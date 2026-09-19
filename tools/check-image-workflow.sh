#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause
#
# The image workflow of a developer: create a sparse image, fork it, mount
# the fork, use it, unmount, read the difference, replay the session on a
# second fork and get the same volume. Every step is one command a person
# types; this gate runs them and checks what each one leaves behind:
#
#   create   mkafsplus --size-mib 256: the host file costs a few hundred KiB
#   fork     cp -c (an APFS clone; cp --reflink=auto on Linux): instant, the
#            base image is untouched afterwards, byte for byte
#   mount    afsplus-mount fork mnt, outside /Volumes, no diskutil
#   use      a drawer of files through the mount, then umount
#   diff     afsplus-image-diff base fork --json lists exactly those objects;
#            afsplus-check says the fork is clean
#   replay   the same session on a second fork: afsplus-image-diff between
#            the two forks shows the same names, links, sizes and bytes
#            (timestamps and commit counters differ, content does not)
#
# macOS with macFUSE for the mount steps; without macFUSE the gate runs the
# other steps and reports the mount ones as not run. Needs the release tools.
#
#   AFSPLUS_WORKFLOW_KEEP=1   keep the work directory

set -u
repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"
target=${CARGO_TARGET_DIR:-"$repo_root/target"}
bin="$target/release"
mountbin=${AFSPLUS_MOUNT_BINARY:-"$target/debug/afsplus-mount"}
work=$(CDPATH= cd -- "$(mktemp -d /tmp/afsplus-workflow.XXXXXX)" && pwd -P)
keep=${AFSPLUS_WORKFLOW_KEEP:-0}
mount_pid=
checks=0; fails=0; notrun=0
say() { printf '%s\n' "$*"; }
ok() { checks=$((checks + 1)); if [ "$1" -ne 0 ]; then fails=$((fails + 1)); say "  FAIL $2"; else say "  ok   $2"; fi; }
skip() { notrun=$((notrun + 1)); say "  not run: $1"; }

for t in mkafsplus afsplus-info afsplus-check afsplus-image-diff; do
    [ -x "$bin/$t" ] || { say "workflow: missing $bin/$t; cargo build --release -p afsplus-tools -p afsplus-check"; exit 69; }
done

# ---- the mount helpers of check-mounted-usage.sh, same rules ----
path_responds() {
    marker=$work/responds.$$; rm -f "$marker"
    ( ls -d "$1" >/dev/null 2>&1; echo done > "$marker" ) & prober=$!
    tries=0
    while [ "$tries" -lt 20 ]; do
        [ -f "$marker" ] && { kill "$prober" 2>/dev/null; return 0; }
        sleep 0.1; tries=$((tries + 1))
    done
    kill -9 "$prober" 2>/dev/null; return 1
}
is_mounted() {
    python3 - "$1" <<'CHECK'
import ctypes, ctypes.util, sys
class Statfs(ctypes.Structure):
    _fields_ = [("head", ctypes.c_char * 72), ("f_fstypename", ctypes.c_char * 16),
                ("f_mntonname", ctypes.c_char * 1024), ("f_mntfromname", ctypes.c_char * 1024),
                ("tail", ctypes.c_uint32 * 8)]
table = ctypes.POINTER(Statfs)()
count = ctypes.CDLL(ctypes.util.find_library("c")).getmntinfo(ctypes.byref(table), 2)
wanted = sys.argv[1].encode()
sys.exit(0 if any(table[i].f_mntonname == wanted for i in range(count)) else 1)
CHECK
}
release_mount() {  # release_mount <mountpoint>
    is_mounted "$1" || return 0
    umount "$1" >/dev/null 2>&1 || umount -f "$1" >/dev/null 2>&1 || true
    tries=0
    while is_mounted "$1"; do
        sleep 1; tries=$((tries + 1))
        [ "$tries" -lt 15 ] || { say "        the volume would not unmount"; return 1; }
    done
}
cleanup() {
    status=$?
    release_mount "$work/mnt"
    [ -z "$mount_pid" ] || wait "$mount_pid" 2>/dev/null || true
    if [ "$keep" = 1 ]; then say "kept: $work"; exit $status; fi
    [ ! -d "$work" ] || rm -r "$work"
    exit $status
}
trap cleanup EXIT HUP INT TERM

# ---- create ----
say "create"
"$bin/mkafsplus" --size-mib 256 --label Workflow "$work/base.afsp" > /dev/null; ok $? "mkafsplus makes a 256 MiB volume"
used_kib=$(du -k "$work/base.afsp" | cut -f1)
[ "$used_kib" -lt 4096 ]; ok $? "the host file is sparse: $used_kib KiB on disk for 256 MiB"

# ---- fork ----
say "fork"
base_sum=$(shasum -a 256 "$work/base.afsp" | cut -c1-64)
if [ "$(uname -s)" = Darwin ]; then cp -c "$work/base.afsp" "$work/fork1.afsp"; else cp --reflink=auto "$work/base.afsp" "$work/fork1.afsp"; fi
ok $? "cp -c clones the image (APFS clone; cp --reflink=auto elsewhere)"
cp "$work/base.afsp" "$work/fork2.afsp" 2>/dev/null || true
[ "$(shasum -a 256 "$work/fork1.afsp" | cut -c1-64)" = "$base_sum" ]; ok $? "the fork starts identical to the base"

# ---- mount, use, unmount: needs macFUSE ----
session() {  # session <image> <mountpoint>: the same drawer every time
    mkdir -p "$2"
    path_responds "$2" || { say "  $2 does not answer; a mount left behind there is blocking it"; return 1; }
    "$mountbin" "$1" "$2" > "$work/mount.log" 2>&1 & mount_pid=$!
    waited=0
    while ! is_mounted "$2"; do
        sleep 1; waited=$((waited + 1))
        [ "$waited" -le 20 ] || { sed 's/^/  /' "$work/mount.log"; return 1; }
    done
    mkdir "$2/Documents" && printf 'hello there\n' > "$2/Documents/note.txt" \
        && mkdir "$2/Documents/Éléments" && printf 'x\n' > "$2/Documents/Éléments/été.txt" \
        && dd if=/dev/zero of="$2/big.bin" bs=1m count=3 2>/dev/null \
        && rm "$2/Documents/Éléments/été.txt"
    rc=$?
    release_mount "$2"; wait "$mount_pid" 2>/dev/null; mount_pid=
    return $rc
}
if [ "$(uname -s)" = Darwin ] && [ -d /Library/Frameworks/macFUSE.framework ] && [ -x "$mountbin" ]; then
    say "mount, use, unmount"
    session "$work/fork1.afsp" "$work/mnt"; ok $? "a session on the first fork: a drawer, a file with an accent, 3 MiB, a delete, unmount"
    [ "$(shasum -a 256 "$work/base.afsp" | cut -c1-64)" = "$base_sum" ]; ok $? "the base image is untouched by the session"
    say "diff and check"
    "$bin/afsplus-check" "$work/fork1.afsp" --json > "$work/check1.json"; ok $? "afsplus-check: the fork is clean"
    "$bin/afsplus-image-diff" "$work/base.afsp" "$work/fork1.afsp" --json > "$work/diff.json"
    python3 - "$work/diff.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
names = {l["name"] for l in d["links"] if l["change"] == "added"}
created = [o for o in d["objects"] if o["change"] == "created"]
sys.exit(0 if {"Documents", "note.txt", "big.bin"} <= names and len(created) >= 3 and not d["partial"] else 1)
PY
    ok $? "afsplus-image-diff base fork lists the drawer, the note and the 3 MiB file as created"
    say "replay"
    session "$work/fork2.afsp" "$work/mnt"; ok $? "the same session on the second fork"
    "$bin/afsplus-image-diff" "$work/fork1.afsp" "$work/fork2.afsp" --json > "$work/replay.json"
    python3 - "$work/replay.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
# Timestamps and content_generation (a commit counter: how many commits a
# session took depends on the commit timer and idle-time cleanup) are not
# content. Names, links, sizes and bytes must be the same.
other = [f for o in d["objects"] for f in o.get("fields", [])
         if f.get("field") not in ("timestamp", "content_generation")]
sys.exit(0 if not d["links"] and not d["renames"] and not other and not d["partial"] else 1)
PY
    ok $? "the two forks hold the same names, links, sizes and bytes: the session replays"
else
    skip "mount, use, unmount, diff and replay need macOS with macFUSE and target/debug/afsplus-mount (cargo build -p afsplus-fuse --features macfuse-mount --bin afsplus-mount)"
fi

say "image-workflow: $checks checks, $fails failures, $notrun not run; work in $work"
[ "$fails" -eq 0 ]
