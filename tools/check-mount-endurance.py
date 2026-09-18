#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""A mounted volume holds up under hours of ordinary use by several programs.

The other mounted checks each last seconds. A leak, a slow drift or a
contention between programs only shows over time, so this keeps three
programs at work on one volume for half an hour by default:

  big      writes files of 5 to 40 MiB, reads each back, renames and deletes it
  tree     builds folders of small files, renames within and across folders,
           rewrites and deletes them
  browser  lists every folder and reads every file it finds, the way a file
           manager does, and times each of those small operations

Every file's bytes follow from its name, so any read anywhere is checked.
At the end every program must report no error, the browser's worst wait for
one small operation must stay under a bound, the space of everything deleted
must be back, and after unmounting the checker must find the image clean.
The mount is outside /Volumes and whatever the run leaves mounted is
released at the end.

  tools/check-mount-endurance.py                    thirty minutes
  AFSPLUS_ENDURANCE_SECONDS=300 tools/check-mount-endurance.py

Exit 0 when every check holds, 1 when one does not, 66 when it could not run.
"""

import ctypes
import ctypes.util
import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
RELAY = "io.macfuse.app.fsmodule.macfuse"
SECONDS = int(os.environ.get("AFSPLUS_ENDURANCE_SECONDS", "1800"))
# The browser's worst wait for one listing, stat or small read. An idle
# volume answers in a few milliseconds. Under this load it waits behind the
# other programs' writes, each made durable before it is answered, and a
# second is where a person starts calling a program frozen.
WORST_WAIT_MS = 1000
# Space the volume may keep after everything is deleted: the directory and
# reclaim structures it grew to hold the load, which later use reuses.
KEPT_KIB = 1024
failures = []

CLIENT = r'''
import hashlib, json, os, random, sys, time
root, role, seconds, seed = sys.argv[1], sys.argv[2], float(sys.argv[3]), int(sys.argv[4])
chance = random.Random(seed)
deadline = time.time() + seconds
report = {"role": role, "operations": 0, "errors": [], "worst_ms": 0.0, "bytes": 0}

def content(name, size):
    block = hashlib.sha256(name.encode()).digest() * 128
    return (block * (size // len(block) + 1))[:size]

def size_of(name):
    return int(name.rsplit("-", 1)[1])

def error(what):
    if len(report["errors"]) < 20:
        report["errors"].append(what)

def write(path, size):
    # Under a temporary name, then renamed into place, as an editor saves:
    # the browser never meets a file half written.
    data = content(os.path.basename(path), size)
    partial = os.path.join(os.path.dirname(path), ".partial-" + os.path.basename(path))
    with open(partial, "wb") as handle:
        for offset in range(0, size, 1 << 20):
            handle.write(data[offset:offset + (1 << 20)])
    os.rename(partial, path)
    report["bytes"] += size

def verify(path):
    name = os.path.basename(path)
    with open(path, "rb") as handle:
        data = handle.read()
    # A renamed file keeps the bytes of the name it was written under.
    written_as = name[len("moved-"):] if name.startswith("moved-") else name
    if data != content(written_as, size_of(name)):
        error(f"{name} reads back {len(data)} bytes that are not what was written")

home = os.path.join(root, role)
os.makedirs(home, exist_ok=True)
counter = 0
while time.time() < deadline:
    counter += 1
    try:
        if role == "big":
            name = f"big{counter}-{chance.randint(5, 40) << 20}"
            path = os.path.join(home, name)
            write(path, size_of(name))
            verify(path)
            moved = os.path.join(home, "moved-" + name)
            os.rename(path, moved)
            verify(moved)
            os.remove(moved)
        elif role == "tree":
            folder = os.path.join(home, f"t{counter % 7}")
            os.makedirs(folder, exist_ok=True)
            for index in range(chance.randint(5, 40)):
                name = f"f{counter}x{index}-{chance.randint(1, 60000)}"
                write(os.path.join(folder, name), size_of(name))
            names = sorted(os.listdir(folder))
            for name in chance.sample(names, min(len(names), 10)):
                verify(os.path.join(folder, name))
            other = os.path.join(home, f"t{(counter + 3) % 7}")
            os.makedirs(other, exist_ok=True)
            for name in chance.sample(names, min(len(names), 5)):
                os.rename(os.path.join(folder, name), os.path.join(other, name))
            for name in chance.sample(sorted(os.listdir(other)), min(len(os.listdir(other)), 8)):
                os.remove(os.path.join(other, name))
            if counter % 11 == 0:
                for name in os.listdir(folder):
                    os.remove(os.path.join(folder, name))
                os.rmdir(folder)
        else:
            for directory, folders, files in os.walk(root):
                for name in files:
                    if name.startswith(".partial-"):
                        continue
                    path = os.path.join(directory, name)
                    started = time.time()
                    try:
                        os.stat(path)
                        if size_of(name) <= 65536:
                            verify(path)
                    except FileNotFoundError:
                        continue  # another program deleted it meanwhile
                    report["worst_ms"] = max(report["worst_ms"], (time.time() - started) * 1000)
                started = time.time()
                os.listdir(directory)
                report["worst_ms"] = max(report["worst_ms"], (time.time() - started) * 1000)
            time.sleep(0.05)
        report["operations"] += 1
    except FileNotFoundError as problem:
        if role != "browser":
            error(f"round {counter}: a file of its own vanished: {problem}")
    except OSError as problem:
        error(f"round {counter}: {problem}")
# Leave the volume as it was found.
if role != "browser":
    for directory, folders, files in os.walk(home, topdown=False):
        for name in files:
            os.remove(os.path.join(directory, name))
        for name in folders:
            os.rmdir(os.path.join(directory, name))
    os.rmdir(home)
print(json.dumps(report), flush=True)
'''


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
    _fields_ = [("head", ctypes.c_char * 72), ("f_fstypename", ctypes.c_char * 16),
                ("f_mntonname", ctypes.c_char * 1024), ("f_mntfromname", ctypes.c_char * 1024),
                ("tail", ctypes.c_uint32 * 8)]


_libc = ctypes.CDLL(ctypes.util.find_library("c"))


def mounted(canonical):
    """Read with MNT_NOWAIT: no filesystem is asked, so a dead mount cannot block it."""
    table = ctypes.POINTER(_Statfs)()
    count = _libc.getmntinfo(ctypes.byref(table), 2)
    return any(table[index].f_mntonname == canonical.encode() for index in range(count))


def free_kib(path):
    stats = os.statvfs(path)
    return stats.f_bfree * stats.f_frsize // 1024


if sys.platform != "darwin":
    fatal("this drives a macFUSE mount and needs macOS")
if not os.path.isdir("/Library/Frameworks/macFUSE.framework"):
    fatal("macFUSE is not installed")

print("building afsplus-mount, mkafsplus and afsplus-check")
for command in (
    ["cargo", "build", "-q", "--release", "-p", "afsplus-tools", "--bin", "mkafsplus"],
    ["cargo", "build", "-q", "--release", "-p", "afsplus-check", "--bin", "afsplus-check"],
    ["cargo", "build", "-q", "--release", "-p", "afsplus-fuse", "--features", "macfuse-mount", "--bin", "afsplus-mount"],
):
    if subprocess.run(command, cwd=ROOT).returncode != 0:
        fatal("cannot build " + command[-1])
target = os.environ.get("CARGO_TARGET_DIR", os.path.join(ROOT, "target"))
MKFS = os.path.join(target, "release", "mkafsplus")
CHECK = os.path.join(target, "release", "afsplus-check")
MOUNT = os.environ.get("AFSPLUS_MOUNT_BINARY", os.path.join(target, "release", "afsplus-mount"))

work = os.path.realpath(tempfile.mkdtemp(prefix="afsplus-endurance-"))
image, mnt = os.path.join(work, "volume.img"), os.path.join(work, "mnt")
os.makedirs(mnt)
relays_before_run = relays()
supervisor = None

try:
    if subprocess.run([MKFS, "--size-mib", "512", "--label", "Endurance", "--force", image],
                      capture_output=True).returncode:
        fatal("cannot format the image")
    supervisor = subprocess.Popen([MOUNT, image, mnt], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
    deadline = time.time() + 40
    while time.time() < deadline and not mounted(mnt) and supervisor.poll() is None:
        time.sleep(0.1)
    if not mounted(mnt):
        fatal("the volume did not mount")
    time.sleep(1)
    free_at_start = free_kib(mnt)

    print(f"\nthree programs at work on the volume for {SECONDS} s")
    clients = [
        subprocess.Popen([sys.executable, "-c", CLIENT, mnt, role, str(SECONDS), str(seed)],
                         stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        for seed, role in enumerate(("big", "tree", "browser"), start=1)
    ]
    reports = []
    for client in clients:
        out, err = client.communicate(timeout=SECONDS + 600)
        try:
            reports.append(json.loads(out.strip().splitlines()[-1]))
        except (ValueError, IndexError):
            reports.append({"role": "?", "operations": 0, "errors": [f"no report: {err.strip()[-300:]}"],
                            "worst_ms": 0, "bytes": 0})
    for report in reports:
        print(f"        {report['role']}: {report['operations']} rounds, "
              f"{report['bytes'] >> 20} MiB written, worst small operation {report['worst_ms']:.0f} ms")
        check(f"{report['role']} met no error", not report["errors"], "; ".join(report["errors"][:5]))
    browser = next((report for report in reports if report["role"] == "browser"), None)
    check(f"the browser never waited {WORST_WAIT_MS} ms for one small operation",
          browser is not None and browser["worst_ms"] < WORST_WAIT_MS,
          f"it waited {browser['worst_ms']:.0f} ms" if browser else "")

    # The space of everything deleted comes back, in the background.
    back = False
    deadline = time.time() + 120
    while time.time() < deadline:
        if free_kib(mnt) >= free_at_start - KEPT_KIB:
            back = True
            break
        time.sleep(1)
    check("the space of everything deleted is back", back,
          f"{free_at_start - free_kib(mnt)} KiB still used of what was free at the start")

    subprocess.run(["umount", mnt], capture_output=True, timeout=30)
    deadline = time.time() + 60
    while time.time() < deadline and supervisor.poll() is None:
        time.sleep(0.2)
    check("the volume unmounts cleanly", supervisor.poll() == 0 and not mounted(mnt))
    result = subprocess.run([CHECK, image], capture_output=True, text=True)
    check("the checker finds the image clean", result.returncode == 0,
          (result.stdout + result.stderr).strip()[-400:])

finally:
    leftover_relays = relays() - relays_before_run
    for pid in leftover_relays:
        os.kill(pid, signal.SIGTERM)
    if leftover_relays:
        time.sleep(1)
    if mounted(mnt):
        try:
            subprocess.run(["umount", mnt], capture_output=True, timeout=30)
        except subprocess.TimeoutExpired:
            pass
    if leftover_relays:
        failures.append("the run left a relay behind and had to release it itself")
    shutil.rmtree(work, ignore_errors=True)

print()
if failures:
    print(f"{len(failures)} check(s) failed")
    sys.exit(1)
print(f"every check held: {SECONDS} s of three programs at work")
