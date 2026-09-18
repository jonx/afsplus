#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""A volume whose process dies must not leave a mount that only a reboot clears.

macFUSE's FSKit backend relays every request to the driver through a helper
process, one per mount. When the driver died while a request was in flight,
that helper waited for an answer that never came: the program waiting could
not even be killed, umount blocked the same way, and the mountpoint stayed
behind with every stat on it blocking until the machine was restarted.

afsplus-mount now supervises the process that serves the volume and releases
that helper when the process ends any other way than a clean unmount. This
checks it with the scenario that used to require a reboot, and with the
ordinary ways a mount ends, all on mountpoints outside /Volumes so that a
failure can never reach the desktop.

It cannot leave a dead mount behind even if the driver regresses: whatever
this run mounted and did not see released is released by hand at the end,
and the run then reports a failure.

  tools/check-mount-driver-death.py

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
import time

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
RELAY = "io.macfuse.app.fsmodule.macfuse"
failures = []


def fatal(reason):
    print(f"cannot run the check: {reason}")
    sys.exit(66)


def check(what, holds, detail=""):
    print(f"  {'ok  ' if holds else 'FAIL'}  {what}" + (f"\n        {detail}" if detail and not holds else ""))
    if not holds:
        failures.append(what)


def relays():
    """Processes whose executable is the relay, matched on its path.

    Not `pgrep -f`: that searches whole command lines and matches any shell
    that mentions the name, and this list decides what gets killed.
    """
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
MNT_NOWAIT = 2


def mounted(mnt):
    """Whether mnt is in the mount table, without asking any filesystem.

    Not `mount`: it asks every mounted filesystem for its state, so one dead
    mount anywhere blocks it for ever, and a check for dead mounts has to keep
    working while one exists. getmntinfo with MNT_NOWAIT reads the cached
    table and asks nothing.
    """
    table = ctypes.POINTER(_Statfs)()
    count = _libc.getmntinfo(ctypes.byref(table), MNT_NOWAIT)
    # The spelling recorded before the volume mounted: resolving the path now
    # would stat the mountpoint, which blocks once the mount is dead.
    wanted = CANONICAL.get(mnt, mnt).encode()
    return any(table[index].f_mntonname == wanted for index in range(count))


def answers(path, wait=3.0):
    """Whether a stat on the path returns at all, rather than blocking."""
    probe = subprocess.Popen(["ls", "-d", path], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    deadline = time.time() + wait
    while time.time() < deadline and probe.poll() is None:
        time.sleep(0.05)
    if probe.poll() is None:
        probe.kill()
        return False
    return True


def ends(process, wait=20.0):
    deadline = time.time() + wait
    while time.time() < deadline and process.poll() is None:
        time.sleep(0.1)
    return process.poll()


if sys.platform != "darwin":
    fatal("this drives a macFUSE mount and needs macOS")
if not os.path.isdir("/Library/Frameworks/macFUSE.framework"):
    fatal("macFUSE is not installed")

print("building afsplus-mount and mkafsplus")
env = dict(os.environ)
for command in (
    ["cargo", "build", "-q", "--release", "-p", "afsplus-tools", "--bin", "mkafsplus"],
    ["cargo", "build", "-q", "--release", "-p", "afsplus-fuse", "--features", "macfuse-mount", "--bin", "afsplus-mount"],
):
    if subprocess.run(command, cwd=ROOT, env=env).returncode != 0:
        fatal("cannot build " + command[-1])
target = os.environ.get("CARGO_TARGET_DIR", os.path.join(ROOT, "target"))
MKFS = os.path.join(target, "release", "mkafsplus")
# A negative control points this at a deliberately broken build, so that the
# source is never left edited while a run is in progress.
MOUNT = os.environ.get("AFSPLUS_MOUNT_BINARY", os.path.join(target, "release", "afsplus-mount"))

work = tempfile.mkdtemp(prefix="afsplus-death-")
CANONICAL = {}
relays_before_run = relays()
started = []


def volume(name):
    image = os.path.join(work, f"{name}.img")
    mnt = os.path.join(work, f"{name}-mnt")
    os.makedirs(mnt)
    CANONICAL[mnt] = os.path.realpath(mnt)  # safe now: nothing is mounted here yet
    if subprocess.run([MKFS, "--size-mib", "16", "--label", name, "--force", image], capture_output=True).returncode:
        fatal(f"cannot format {name}")
    supervisor = subprocess.Popen([MOUNT, image, mnt], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    started.append(mnt)
    deadline = time.time() + 20
    while time.time() < deadline and not mounted(mnt):
        if supervisor.poll() is not None:
            fatal(f"{name} did not mount: {supervisor.stdout.read().strip()}")
        time.sleep(0.1)
    if not mounted(mnt):
        fatal(f"{name} did not appear in the mount table")
    return supervisor, mnt


def serving_process(supervisor):
    out = subprocess.run(["pgrep", "-P", str(supervisor.pid)], capture_output=True, text=True).stdout.split()
    return int(out[0]) if out else None


try:
    print("\nthe process serving a volume dies while a program is waiting on it")
    supervisor, mnt = volume("dying")
    with open(os.path.join(mnt, "a.txt"), "w") as handle:
        handle.write("hello")
    served_by = serving_process(supervisor)
    if served_by is None:
        fatal("afsplus-mount started no serving process: is it the supervised build?")
    os.kill(served_by, signal.SIGSTOP)
    reader = subprocess.Popen(["cat", os.path.join(mnt, "a.txt")], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(2)
    check("a program reading the volume waits while its process is stopped", reader.poll() is None)
    os.kill(served_by, signal.SIGKILL)
    ends(reader)
    check("the waiting program is released when that process dies", reader.poll() is not None,
          "it would have stayed blocked until a reboot")
    status = ends(supervisor)
    said = supervisor.stdout.read() if supervisor.poll() is not None else ""
    check("afsplus-mount says the volume process was killed", status not in (None, 0) and "killed" in said, said.strip())
    check("the mountpoint answers afterwards", answers(mnt))
    check("the dead volume is no longer mounted", not mounted(mnt))

    print("\nasking afsplus-mount to stop unmounts cleanly")
    for stop in (signal.SIGTERM, signal.SIGINT):
        supervisor, mnt = volume(f"stop-{stop.name.lower()}")
        with open(os.path.join(mnt, "kept.txt"), "w") as handle:
            handle.write("kept")
        os.kill(supervisor.pid, stop)
        check(f"{stop.name} ends it as a clean unmount", ends(supervisor) == 0 and not mounted(mnt))
        check(f"{stop.name} leaves the mountpoint answering", answers(mnt))

    print("\none volume dying leaves another alone")
    alive_supervisor, alive = volume("alive")
    dying_supervisor, dying = volume("neighbour")
    os.kill(serving_process(dying_supervisor), signal.SIGKILL)
    ends(dying_supervisor)
    with open(os.path.join(alive, "after.txt"), "w") as handle:
        handle.write("still here")
    with open(os.path.join(alive, "after.txt")) as handle:
        check("the other volume still writes and reads back", handle.read() == "still here")
    check("the other volume is still mounted", mounted(alive))
    subprocess.run(["umount", alive], capture_output=True, timeout=15)
    check("and it unmounts cleanly afterwards", ends(alive_supervisor) == 0 and not mounted(alive))

finally:
    # Whatever this run left mounted, or any relay it started that is still
    # alive, is released here so that a regression cannot cost a reboot.
    # Relays first: until they go, anything that touches a dead mount blocks.
    leftover_relays = relays() - relays_before_run
    for pid in leftover_relays:
        os.kill(pid, signal.SIGTERM)
    time.sleep(1)
    leftover_mounts = [mnt for mnt in started if mounted(mnt)]
    for mnt in leftover_mounts:
        try:
            subprocess.run(["umount", CANONICAL[mnt]], capture_output=True, timeout=15)
        except subprocess.TimeoutExpired:
            pass
    if leftover_relays or leftover_mounts:
        failures.append("the run left relays or mounts behind and had to release them itself")
        print(f"\n  FAIL  released by hand: {len(leftover_relays)} relays, {len(leftover_mounts)} mounts")
    shutil.rmtree(work, ignore_errors=True)

print()
if failures:
    print(f"{len(failures)} check(s) failed")
    sys.exit(1)
print("every check held: no way of ending a mount left one behind")
