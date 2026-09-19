#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""Memory-mapped files on a mounted volume, under parallel page faults.

A program that maps a file gets its bytes through the kernel's page cache,
which asks the driver for whole pages in whatever order the faults come,
from as many processes as touch the map. This mounts a volume, writes a
32 MiB file with a known pattern, maps it from eight processes at once that
each fault every page in their own random order, and requires every byte
to be right; then maps it writable and shared, writes through the map from
two processes, msyncs, and requires the bytes to be on the volume after the
unmount, read back through the core and not through the mount. The mount
is outside /Volumes, released at the end, no diskutil.

  tools/check-mount-mmap.py

Exit 0 when every check holds, 1 when one does not, 66 when it could not run.
"""
import ctypes
import ctypes.util
import hashlib
import mmap
import multiprocessing
import os
import random
import shutil
import signal
import subprocess
import sys
import tempfile
import time

# The workers only map a file: fork, so the module body does not run again
# in each of them, as macOS's spawn would make it.
multiprocessing.set_start_method("fork")
ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
RELAY = "io.macfuse.app.fsmodule.macfuse"
SIZE = 32 * 1024 * 1024
PAGE = 4096
READERS = 8
failures = []


def fatal(reason):
    print(f"cannot run the check: {reason}")
    sys.exit(66)


def check(what, holds, detail=""):
    print(f"  {'ok  ' if holds else 'FAIL'}  {what}" + (f"\n        {detail}" if detail and not holds else ""))
    if not holds:
        failures.append(what)


def relays():
    out = subprocess.run(["ps", "-axo", "pid=,comm="], capture_output=True, text=True).stdout
    found = set()
    for line in out.splitlines():
        parts = line.strip().split(None, 1)
        if len(parts) == 2 and (parts[1] == RELAY or parts[1].endswith("/" + RELAY)):
            found.add(int(parts[0]))
    return found


class _Statfs(ctypes.Structure):
    _fields_ = [
        ("f_bsize", ctypes.c_uint32), ("f_iosize", ctypes.c_int32),
        ("f_blocks", ctypes.c_uint64), ("f_bfree", ctypes.c_uint64),
        ("f_bavail", ctypes.c_uint64), ("f_files", ctypes.c_uint64),
        ("f_ffree", ctypes.c_uint64), ("f_fsid", ctypes.c_int32 * 2),
        ("f_owner", ctypes.c_uint32), ("f_type", ctypes.c_uint32),
        ("f_flags", ctypes.c_uint32), ("f_fssubtype", ctypes.c_uint32),
        ("f_fstypename", ctypes.c_char * 16), ("f_mntonname", ctypes.c_char * 1024),
        ("f_mntfromname", ctypes.c_char * 1024), ("f_flags_ext", ctypes.c_uint32),
        ("f_reserved", ctypes.c_uint32 * 7),
    ]


_libc = ctypes.CDLL(ctypes.util.find_library("c"))


def mounted(canonical):
    table = ctypes.POINTER(_Statfs)()
    count = _libc.getmntinfo(ctypes.byref(table), 2)
    return any(table[index].f_mntonname == canonical.encode() for index in range(count))


def pattern(page):
    """Page `page` of the file: its number, then a byte sequence from it."""
    seed = (page * 2654435761) & 0xFFFFFFFF
    head = seed.to_bytes(4, "little")
    body = bytes((seed >> (8 * (i % 4))) & 0xFF ^ (i & 0xFF) for i in range(PAGE - 4))
    return head + body


def reader(path, seed, result):
    """Fault every page in a random order and count the pages that are wrong."""
    order = list(range(SIZE // PAGE))
    random.Random(seed).shuffle(order)
    wrong = 0
    with open(path, "rb") as handle:
        view = mmap.mmap(handle.fileno(), 0, access=mmap.ACCESS_READ)
        try:
            for page in order:
                if view[page * PAGE:(page + 1) * PAGE] != pattern(page):
                    wrong += 1
        finally:
            view.close()
    result.put((seed, wrong))


def writer(path, pages, stamp):
    """Write a stamp into the first bytes of the given pages through a shared map."""
    with open(path, "r+b") as handle:
        view = mmap.mmap(handle.fileno(), 0, access=mmap.ACCESS_WRITE)
        try:
            for page in pages:
                view[page * PAGE:page * PAGE + len(stamp)] = stamp
            view.flush()
        finally:
            view.close()


if sys.platform != "darwin":
    fatal("this drives a macFUSE mount and needs macOS")
if not os.path.isdir("/Library/Frameworks/macFUSE.framework"):
    fatal("macFUSE is not installed")
print("building afsplus-mount, mkafsplus and afsplus-extract")
for command in (
    ["cargo", "build", "-q", "--release", "-p", "afsplus-tools", "--bin", "mkafsplus", "--bin", "afsplus-extract"],
    ["cargo", "build", "-q", "--release", "-p", "afsplus-fuse", "--features", "macfuse-mount", "--bin", "afsplus-mount"],
):
    if subprocess.run(command, cwd=ROOT).returncode != 0:
        fatal("cannot build " + command[-1])
target = os.environ.get("CARGO_TARGET_DIR", os.path.join(ROOT, "target"))
MKFS = os.path.join(target, "release", "mkafsplus")
EXTRACT = os.path.join(target, "release", "afsplus-extract")
MOUNT = os.environ.get("AFSPLUS_MOUNT_BINARY", os.path.join(target, "release", "afsplus-mount"))

work = os.path.realpath(tempfile.mkdtemp(prefix="afsplus-mmap-"))
image, mnt = os.path.join(work, "volume.img"), os.path.join(work, "mnt")
os.makedirs(mnt)
relays_before_run = relays()
supervisor = None
try:
    if subprocess.run([MKFS, "--size-mib", "128", "--label", "Mapped", "--force", image], capture_output=True).returncode:
        fatal("cannot format the image")
    supervisor = subprocess.Popen([MOUNT, image, mnt], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    deadline = time.time() + 20
    while time.time() < deadline and not mounted(mnt):
        if supervisor.poll() is not None:
            fatal("the volume did not mount")
        time.sleep(0.1)
    if not mounted(mnt):
        fatal("the volume did not appear in the mount table")

    path = os.path.join(mnt, "mapped.bin")
    print(f"\nwriting {SIZE >> 20} MiB with a per-page pattern")
    with open(path, "wb") as handle:
        for page in range(SIZE // PAGE):
            handle.write(pattern(page))
    subprocess.run(["sync"])

    print(f"{READERS} processes map it and fault every page in their own order")
    results = multiprocessing.Queue()
    started = time.time()
    procs = [multiprocessing.Process(target=reader, args=(path, seed, results)) for seed in range(READERS)]
    for p in procs:
        p.start()
    for p in procs:
        p.join(120)
    outcomes = [results.get(timeout=1) for _ in procs]
    elapsed = time.time() - started
    check("every reader finished", all(p.exitcode == 0 for p in procs))
    check("every page read through the map is right in every process",
          all(wrong == 0 for _, wrong in outcomes), f"wrong pages per reader: {sorted(outcomes)}")
    pages = SIZE // PAGE * READERS
    print(f"        {pages} page faults in {elapsed:.1f} s, {pages / elapsed:.0f} pages/s")

    # Control: a reader must notice a page that is not the pattern.
    with open(path, "r+b") as handle:
        handle.seek(1234 * PAGE)
        handle.write(b"not the pattern")
    control = multiprocessing.Queue()
    p = multiprocessing.Process(target=reader, args=(path, 99, control))
    p.start(); p.join(120)
    _, wrong = control.get(timeout=1)
    check("control: one page changed through write() is one wrong page to a reader", wrong == 1, f"{wrong} wrong")
    with open(path, "r+b") as handle:
        handle.seek(1234 * PAGE)
        handle.write(pattern(1234)[:len(b"not the pattern")])

    print("two processes write through a shared map, msync, and the file is read back")
    stamp_a, stamp_b = b"A-side", b"B-side"
    even = list(range(0, SIZE // PAGE, 2))
    odd = list(range(1, SIZE // PAGE, 2))
    writers = [multiprocessing.Process(target=writer, args=(path, even, stamp_a)),
               multiprocessing.Process(target=writer, args=(path, odd, stamp_b))]
    for p in writers:
        p.start()
    for p in writers:
        p.join(120)
    check("both writers finished", all(p.exitcode == 0 for p in writers))
    with open(path, "rb") as handle:
        data = handle.read()
    ok_through_read = all(data[page * PAGE:page * PAGE + 6] == (stamp_a if page % 2 == 0 else stamp_b)
                          and data[page * PAGE + 6:(page + 1) * PAGE] == pattern(page)[6:]
                          for page in range(SIZE // PAGE))
    check("read() after msync sees both processes' writes and nothing else changed", ok_through_read)
    expected = hashlib.sha256(data).hexdigest()

    subprocess.run(["umount", mnt], capture_output=True, timeout=15)
    deadline = time.time() + 20
    while time.time() < deadline and supervisor.poll() is None:
        time.sleep(0.1)
    check("the volume unmounts cleanly", supervisor.poll() == 0 and not mounted(mnt))

    # The truth: what the volume holds, read by the core, not the mount.
    out = os.path.join(work, "out")   # afsplus-extract makes it
    extracted = subprocess.run([EXTRACT, image, out], capture_output=True, text=True)
    # afsplus-extract writes object-<id>.bin files and a manifest that names
    # them: the link record gives the name (hex), the content record the file.
    import json
    back = os.path.join(out, "missing")
    object_id = None
    try:
        for line in open(os.path.join(out, "manifest.jsonl")):
            record = json.loads(line)
            if record["record"] == "link" and bytes.fromhex(record["name_hex"]) == b"mapped.bin":
                object_id = record["object_id"]
            if record["record"] == "content" and record["object_id"] == object_id:
                back = os.path.join(out, record["file"])
    except OSError:
        pass
    on_disk = hashlib.sha256(open(back, "rb").read()).hexdigest() if os.path.exists(back) else None
    detail = f"extract rc {extracted.returncode}: {extracted.stderr[:200]}; file {back if os.path.exists(back) else 'not found'}"
    if os.path.exists(back) and on_disk != expected:
        got = open(back, "rb").read()
        stale = [page for page in range(SIZE // PAGE)
                 if got[page * PAGE:(page + 1) * PAGE] != data[page * PAGE:(page + 1) * PAGE]]
        detail += f"; {len(got)} bytes, {len(stale)} pages differ, first {stale[:5]}"
        if stale:
            page = stale[0]
            detail += f"; page {page} holds {got[page * PAGE:page * PAGE + 8]!r} for {data[page * PAGE:page * PAGE + 8]!r}"
    check("after the unmount the core reads exactly the bytes the maps left", on_disk == expected, detail)
finally:
    leftover_relays = relays() - relays_before_run
    for pid in leftover_relays:
        os.kill(pid, signal.SIGTERM)
    if leftover_relays:
        time.sleep(1)
    if mounted(mnt):
        try:
            subprocess.run(["umount", mnt], capture_output=True, timeout=15)
        except subprocess.TimeoutExpired:
            pass
    if leftover_relays:
        failures.append("the run left a relay behind and had to release it itself")
    shutil.rmtree(work, ignore_errors=True)

print()
if failures:
    print(f"{len(failures)} check(s) failed")
    sys.exit(1)
print("every check held: mapped reads under parallel faults and shared mapped writes are exact")
