#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""A slow operation on a mounted volume must not freeze its other clients.

On macOS, fuser answers one request at a time, so whatever one request does,
every other client waits for. Deleting a fragmented file used to be such a
request: the driver ran the space-returning maintenance on the close and the
release, under the lock every request takes, and a listing in another program
waited for all of it. Maintenance now runs on its own thread, one transaction
per turn of the lock, so a request waits at most for one transaction.

This writes two files in alternation so that each ends up in many small
extents, deletes one while another program keeps listing a directory and
reading a small file, and requires that program's worst wait to stay short.
It also requires that the deleted file's space is given back, so that making
the delete fast by never doing the work cannot pass. The mount is outside
/Volumes, and whatever the run leaves mounted is released at the end.

  tools/check-mount-responsiveness.py

Exit 0 when every check holds, 1 when one does not, 66 when it could not run.
"""

import ctypes
import ctypes.util
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
RELAY = "io.macfuse.app.fsmodule.macfuse"
# The other program's worst wait for one listing or one small read while the
# delete and its maintenance run. An idle volume answers in about 20 ms; the
# driver that ran maintenance inside the request made it wait over 200 ms.
WORST_WAIT_MS = 100
# How long the deleted file's space may take to come back.
SPACE_BACK_S = 10
FRAGMENTS = 1500
failures = []


def fatal(reason):
    print(f"cannot run the check: {reason}")
    sys.exit(66)


def check(what, holds, detail=""):
    print(f"  {'ok  ' if holds else 'FAIL'}  {what}" + (f"\n        {detail}" if detail and not holds else ""))
    if not holds:
        failures.append(what)


def relays():
    """Relay processes, matched on their executable path and never on `pgrep -f`."""
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
    """Whether the path is in the mount table, read with MNT_NOWAIT so that no
    filesystem is asked anything and a dead mount elsewhere cannot block it."""
    table = ctypes.POINTER(_Statfs)()
    count = _libc.getmntinfo(ctypes.byref(table), 2)
    return any(table[index].f_mntonname == canonical.encode() for index in range(count))


if sys.platform != "darwin":
    fatal("this drives a macFUSE mount and needs macOS")
if not os.path.isdir("/Library/Frameworks/macFUSE.framework"):
    fatal("macFUSE is not installed")

print("building afsplus-mount and mkafsplus")
for command in (
    ["cargo", "build", "-q", "--release", "-p", "afsplus-tools", "--bin", "mkafsplus"],
    ["cargo", "build", "-q", "--release", "-p", "afsplus-fuse", "--features", "macfuse-mount", "--bin", "afsplus-mount"],
):
    if subprocess.run(command, cwd=ROOT).returncode != 0:
        fatal("cannot build " + command[-1])
target = os.environ.get("CARGO_TARGET_DIR", os.path.join(ROOT, "target"))
MKFS = os.path.join(target, "release", "mkafsplus")
# A negative control points this at an older build.
MOUNT = os.environ.get("AFSPLUS_MOUNT_BINARY", os.path.join(target, "release", "afsplus-mount"))

work = os.path.realpath(tempfile.mkdtemp(prefix="afsplus-responsive-"))
image, mnt = os.path.join(work, "volume.img"), os.path.join(work, "mnt")
os.makedirs(mnt)
relays_before_run = relays()
supervisor = None

try:
    if subprocess.run([MKFS, "--size-mib", "256", "--label", "Responsive", "--force", image], capture_output=True).returncode:
        fatal("cannot format the image")
    supervisor = subprocess.Popen([MOUNT, image, mnt], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    deadline = time.time() + 20
    while time.time() < deadline and not mounted(mnt):
        if supervisor.poll() is not None:
            fatal("the volume did not mount")
        time.sleep(0.1)
    if not mounted(mnt):
        fatal("the volume did not appear in the mount table")

    print(f"\nwriting two files in alternation, {FRAGMENTS} blocks each")
    chunk = b"x" * 4096
    with open(os.path.join(mnt, "a.bin"), "wb") as a, open(os.path.join(mnt, "b.bin"), "wb") as b:
        for _ in range(FRAGMENTS):
            for handle in (a, b):
                handle.write(chunk)
                handle.flush()
                os.fsync(handle.fileno())
    os.makedirs(os.path.join(mnt, "dir"))
    for index in range(20):
        with open(os.path.join(mnt, "dir", f"f{index}"), "w") as handle:
            handle.write("x")
    with open(os.path.join(mnt, "small.txt"), "w") as handle:
        handle.write("hello")
    time.sleep(1)
    free_before = os.statvfs(mnt).f_bfree

    worst = {"listing": 0.0, "small read": 0.0}
    stop = threading.Event()

    def other_program():
        while not stop.is_set():
            started = time.time()
            os.listdir(os.path.join(mnt, "dir"))
            worst["listing"] = max(worst["listing"], time.time() - started)
            started = time.time()
            with open(os.path.join(mnt, "small.txt")) as handle:
                handle.read()
            worst["small read"] = max(worst["small read"], time.time() - started)
            time.sleep(0.01)

    print("\ndeleting one of them while another program lists and reads")
    other = threading.Thread(target=other_program)
    other.start()
    time.sleep(0.5)
    os.remove(os.path.join(mnt, "a.bin"))
    deleted_at = time.time()
    back_after = None
    while time.time() - deleted_at < SPACE_BACK_S:
        if os.statvfs(mnt).f_bfree >= free_before + FRAGMENTS * 4096 // os.statvfs(mnt).f_frsize * 9 // 10:
            back_after = time.time() - deleted_at
            break
        time.sleep(0.05)
    time.sleep(1)
    stop.set()
    other.join()
    for what, seconds in worst.items():
        check(f"the other program's worst {what} waits under {WORST_WAIT_MS} ms",
              seconds * 1000 < WORST_WAIT_MS, f"it waited {seconds * 1000:.0f} ms")
    check(f"the deleted file's space is back within {SPACE_BACK_S} s", back_after is not None)
    print(f"        worst listing {worst['listing'] * 1000:.0f} ms, worst small read "
          f"{worst['small read'] * 1000:.0f} ms, space back after "
          + (f"{back_after:.1f} s" if back_after is not None else "never"))

    subprocess.run(["umount", mnt], capture_output=True, timeout=15)
    deadline = time.time() + 20
    while time.time() < deadline and supervisor.poll() is None:
        time.sleep(0.1)
    check("the volume unmounts cleanly", supervisor.poll() == 0 and not mounted(mnt))

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
print("every check held: a slow delete left the other program responsive")
