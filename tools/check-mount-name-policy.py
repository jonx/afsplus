#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""What a mounted volume tells programs about case must be what it does.

A volume is formatted case-sensitive or case-insensitive. Programs that care,
git and rsync among them, do not probe: they ask the system with
pathconf(_PC_CASE_SENSITIVE). The library under the macOS driver used to
declare every volume case-insensitive, so a case-sensitive volume, which keeps
Name.txt and NAME.TXT apart, told those programs the opposite.

This formats one volume of each policy, mounts each outside /Volumes, and
requires the declaration and the behaviour to agree on both.

  tools/check-mount-name-policy.py

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
PC_CASE_SENSITIVE = 11
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
    _fields_ = [("head", ctypes.c_char * 72), ("f_fstypename", ctypes.c_char * 16),
                ("f_mntonname", ctypes.c_char * 1024), ("f_mntfromname", ctypes.c_char * 1024),
                ("tail", ctypes.c_uint32 * 8)]


_libc = ctypes.CDLL(ctypes.util.find_library("c"))


def mounted(canonical):
    """Read with MNT_NOWAIT: no filesystem is asked, so a dead mount cannot block it."""
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

work = os.path.realpath(tempfile.mkdtemp(prefix="afsplus-case-"))
relays_before_run = relays()
started = []

try:
    for policy, sensitive in (("--case-sensitive", True), ("--case-insensitive", False)):
        name = policy.lstrip("-")
        image, mnt = os.path.join(work, f"{name}.img"), os.path.join(work, f"{name}-mnt")
        os.makedirs(mnt)
        if subprocess.run([MKFS, "--size-mib", "16", "--label", "Case", policy, "--force", image],
                          capture_output=True).returncode:
            fatal(f"cannot format a {name} volume")
        supervisor = subprocess.Popen([MOUNT, image, mnt], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        started.append(mnt)
        deadline = time.time() + 20
        while time.time() < deadline and not mounted(mnt):
            if supervisor.poll() is not None:
                fatal(f"the {name} volume did not mount")
            time.sleep(0.1)
        if not mounted(mnt):
            fatal(f"the {name} volume did not appear in the mount table")

        print(f"\na {name} volume")
        with open(os.path.join(mnt, "Name.txt"), "w") as handle:
            handle.write("first")
        try:
            with open(os.path.join(mnt, "NAME.TXT"), "x") as handle:
                handle.write("second")
            kept_apart = True
        except FileExistsError:
            kept_apart = False
        check(("keeps" if sensitive else "folds") + " Name.txt and NAME.TXT"
              + (" apart" if sensitive else " into one file"), kept_apart == sensitive)
        declared = os.pathconf(mnt, PC_CASE_SENSITIVE)
        check("tells programs it is " + ("case-sensitive" if sensitive else "case-insensitive"),
              declared == (1 if sensitive else 0), f"pathconf(_PC_CASE_SENSITIVE) answered {declared}")

        subprocess.run(["umount", mnt], capture_output=True, timeout=15)
        deadline = time.time() + 20
        while time.time() < deadline and supervisor.poll() is None:
            time.sleep(0.1)
        check("unmounts cleanly", supervisor.poll() == 0 and not mounted(mnt))

finally:
    leftover_relays = relays() - relays_before_run
    for pid in leftover_relays:
        os.kill(pid, signal.SIGTERM)
    if leftover_relays:
        time.sleep(1)
    for mnt in started:
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
print("every check held: each volume says about case what it does")
