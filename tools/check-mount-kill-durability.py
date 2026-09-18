#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""A driver killed while programs write keeps everything they fsync'd.

A program that calls fsync and gets success has been promised that the data
is on the volume. This holds the mounted driver to that promise at the worst
moment: while a writer creates files, writes them in pieces and fsyncs them,
the process serving the volume is killed with SIGKILL, at a different instant
each round. After every kill the image must pass the checker, and on the next
mount every file the writer had reported as fsync'd must read back byte for
byte. Rounds continue on the same image, so later rounds start from a volume
that has already survived kills.

Mounts are outside /Volumes, the mount table is read without asking any
filesystem, and whatever the run leaves mounted is released at the end.

  tools/check-mount-kill-durability.py

Exit 0 when every check holds, 1 when one does not, 66 when it could not run.
"""

import ctypes
import ctypes.util
import hashlib
import os
import plistlib
import random
import shutil
import signal
import subprocess
import sys
import tempfile
import time

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
RELAY = "io.macfuse.app.fsmodule.macfuse"
# macOS's own record of FSKit mounts. A mount it did not see end stays here,
# and every later mount at that path is refused until a restart.
MOUNT_RECORD = "/Library/Application Support/livefsd/settings.plist"
ROUNDS = 8
failures = []

# The writer runs as its own program, so that it is a client of the volume
# like any other and its fsync goes through the kernel. It reports a file only
# after fsync has returned.
WRITER = r'''
import hashlib, os, sys, time
mnt, prefix, seed = sys.argv[1], sys.argv[2], int(sys.argv[3])
# Only while the volume is mounted: once it is gone, the same path is a plain
# directory, and files written there would be counted as the volume's.
volume_device = os.stat(mnt).st_dev
if volume_device == os.stat(os.path.dirname(mnt)).st_dev:
    sys.exit(0)
state = seed
def chunk(length):
    global state
    state = (state * 6364136223846793005 + 1442695040888963407) % (1 << 64)
    return (state.to_bytes(8, "little") * (length // 8 + 1))[:length]
index = 0
while True:
    name = f"{prefix}-{index}"
    size = 1 + (state % 300_000)
    data = b"".join(chunk(min(40_000, size - offset)) for offset in range(0, size, 40_000))
    try:
        if os.stat(mnt).st_dev != volume_device:
            sys.exit(0)
        fd = os.open(os.path.join(mnt, name), os.O_CREAT | os.O_WRONLY | os.O_TRUNC, 0o644)
        for offset in range(0, len(data), 40_000):
            os.write(fd, data[offset:offset + 40_000])
        os.fsync(fd)
        # Owed from here: fsync returned. Reported before the close, which
        # would commit the file by itself and hide a driver that does not.
        print(name, hashlib.sha256(data).hexdigest(), flush=True)
        os.close(fd)
    except OSError:
        sys.exit(0)
    # A pause after saving, as a person makes: a kill then lands after the
    # fsync and before anything else would have committed the file anyway.
    time.sleep((state % 150) / 1000)
    index += 1
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


def recorded(path):
    """Whether macOS still records a mount at the path; read only."""
    try:
        with open(MOUNT_RECORD, "rb") as handle:
            mounts = plistlib.load(handle).get("mounts", [])
    except (OSError, plistlib.InvalidFileException):
        return False
    return any(entry.get("mountedOn") == path for entry in mounts)


def wait_for(condition, seconds):
    deadline = time.time() + seconds
    while time.time() < deadline:
        if condition():
            return True
        time.sleep(0.1)
    return condition()


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
# A negative control points this at a deliberately broken build.
MOUNT = os.environ.get("AFSPLUS_MOUNT_BINARY", os.path.join(target, "release", "afsplus-mount"))

work = os.path.realpath(tempfile.mkdtemp(prefix="afsplus-kill-"))
image = os.path.join(work, "volume.img")
# Every mount gets a path of its own. Should a kill leave macOS's record of a
# mount behind, the next mount is then not refused, which would stop the run
# and raise a macFUSE alert on the desktop; the record is checked directly.
mounts_made = []


def fresh_mountpoint():
    path = os.path.join(work, f"mnt-{len(mounts_made)}")
    os.makedirs(path)
    mounts_made.append(path)
    return path
relays_before_run = relays()
promised = {}  # name -> sha256 of what fsync acknowledged
left_recorded = []
chance = random.Random(int(os.environ.get("AFSPLUS_KILL_SEED", "20260918")))


def mount():
    mnt = fresh_mountpoint()
    # What afsplus-mount says goes to a file of its own, shown when a check fails.
    said = open(mnt + ".said", "w")
    supervisor = subprocess.Popen([MOUNT, image, mnt], stdout=said, stderr=subprocess.STDOUT)
    if not wait_for(lambda: mounted(mnt) or supervisor.poll() is not None, 40) or not mounted(mnt):
        fatal("the volume did not mount")
    return supervisor, mnt


def verify(label):
    """Every promised file reads back, on a fresh mount."""
    supervisor, mnt = mount()
    missing, differing = [], []
    for name, digest in sorted(promised.items()):
        try:
            with open(os.path.join(mnt, name), "rb") as handle:
                if hashlib.sha256(handle.read()).hexdigest() != digest:
                    differing.append(name)
        except OSError:
            missing.append(name)
    check(f"{label}: all {len(promised)} fsync'd files read back byte for byte",
          not missing and not differing,
          f"missing {missing[:5]}, different {differing[:5]}")
    subprocess.run(["umount", mnt], capture_output=True, timeout=15)
    wait_for(lambda: supervisor.poll() is not None, 20)
    check(f"{label}: a clean unmount leaves no record behind", wait_for(lambda: not recorded(mnt), 5))


try:
    if subprocess.run([MKFS, "--size-mib", "256", "--label", "Kill", "--force", image],
                      capture_output=True).returncode:
        fatal("cannot format the image")

    for round_number in range(ROUNDS):
        delay = chance.uniform(0.3, 4.0)
        print(f"\nround {round_number + 1}: killing the driver {delay:.2f} s into the writing")
        supervisor, mnt = mount()
        serving = subprocess.run(["pgrep", "-P", str(supervisor.pid)], capture_output=True, text=True).stdout.split()
        if not serving:
            fatal("afsplus-mount started no serving process")
        writer = subprocess.Popen([sys.executable, "-c", WRITER, mnt, f"r{round_number}", str(round_number + 1)],
                                  stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        time.sleep(delay)
        os.kill(int(serving[0]), signal.SIGKILL)
        released_at = time.time()
        released = wait_for(lambda: supervisor.poll() is not None and not mounted(mnt), 40)
        with open(mnt + ".said") as said:
            check("the mount is released after the kill", released,
                  f"after {time.time() - released_at:.0f} s: afsplus-mount "
                  + ("is still running" if supervisor.poll() is None else f"ended with {supervisor.poll()}")
                  + ", mounted" * mounted(mnt) + "; it said: " + said.read().strip().replace("\n", " | ")[-400:])
        print(f"        released after {time.time() - released_at:.1f} s")
        if not wait_for(lambda: not recorded(mnt), 30):
            # macOS can keep its record of a mount whose relay had to be
            # terminated; see testing/mounted-volume-testing.md. Reported,
            # not failed: data safety is what this check holds, and every
            # mount here has a path of its own.
            left_recorded.append(mnt)
        if not wait_for(lambda: writer.poll() is not None, 10):
            writer.kill()
        reported = [line.split() for line in writer.stdout.read().splitlines() if len(line.split()) == 2]
        promised.update({name: digest for name, digest in reported})
        print(f"        the writer had {len(reported)} files fsync'd when the driver died")
        result = subprocess.run([CHECK, image], capture_output=True, text=True)
        check("the checker finds the image clean", result.returncode == 0,
              (result.stdout + result.stderr).strip()[-400:])
        verify("after the kill")

    if left_recorded:
        print(f"\nnote: macOS still records {len(left_recorded)} of the {ROUNDS} killed mounts; "
              "those paths refuse a new mount until its file system daemon restarts")
    total = len(promised)
    check("the rounds fsync'd enough files to mean something", total >= ROUNDS * 3, f"only {total}")

finally:
    leftover_relays = relays() - relays_before_run
    for pid in leftover_relays:
        os.kill(pid, signal.SIGTERM)
    if leftover_relays:
        time.sleep(1)
    for mnt in mounts_made:
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
print(f"every check held: {ROUNDS} kills, nothing that was fsync'd was lost")
