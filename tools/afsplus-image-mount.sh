#!/bin/sh
# Mount a stopped AFS+ image on this Mac, and unmount it again.
#
#   tools/afsplus-image-mount.sh mount [--read-only] IMAGE MOUNTPOINT
#   tools/afsplus-image-mount.sh unmount [--force] MOUNTPOINT
#
# `mount` refuses an image that any process holds open (a running VM), checks
# the image, starts afsplus-mount in the background and returns once the
# volume is in the mount table. `unmount` unmounts, waits for the driver to
# write its last state and runs the checker on the image again.
#
# Needs macOS with macFUSE 5.4.0 or later and its FSKit module enabled
# (docs/macos-fskit-activation.md). The how-to is in
# testing/image-workflow.md#copying-files-in-and-out-of-a-stopped-image.
set -eu

here=$(cd "$(dirname "$0")/.." && pwd -P)
bin=${AFSPLUS_BIN:-$here/target/release}
state_dir=${AFSPLUS_MOUNT_STATE:-$HOME/.cache/afsplus-mount}

say() { printf 'afsplus-image-mount: %s\n' "$*" >&2; }
die() { say "$*"; exit 1; }
usage() {
    die "usage: $0 mount [--read-only] IMAGE MOUNTPOINT | unmount [--force] MOUNTPOINT"
}

[ "$(uname -s)" = Darwin ] || die "this helper drives macFUSE and needs macOS; on Linux run afsplus-mount directly"

# Read from the kernel's cached mount table with MNT_NOWAIT. `mount`, `df` and
# `stat` ask the mounted file system itself and block for ever on a dead one.
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

# The spelling the mount table uses (/tmp is /private/tmp), resolved through
# the parent so that a dead mount at the path itself is never visited.
listed_path() {
    parent=$(cd "$(dirname "$1")" && pwd -P) || die "no such directory: $(dirname "$1")"
    printf '%s/%s\n' "${parent%/}" "$(basename "$1")"
}

state_file() {
    printf '%s/%s\n' "$state_dir" "$(printf '%s' "$1" | tr '/' '%')"
}

ensure_tools() {
    if [ ! -x "$bin/afsplus-mount" ]; then
        say "building afsplus-mount"
        (cd "$here" && cargo build --release -p afsplus-fuse --features macfuse-mount --bin afsplus-mount) >&2
    fi
    if [ ! -x "$bin/afsplus-check" ]; then
        say "building afsplus-check"
        (cd "$here" && cargo build --release -p afsplus-check) >&2
    fi
}

check_image() {
    if "$bin/afsplus-check" "$1" >&2; then
        return 0
    fi
    return 1
}

do_mount() {
    mode=
    if [ "${1:-}" = --read-only ]; then
        mode=--read-only
        shift
    fi
    [ $# -eq 2 ] || usage
    image=$1
    [ -f "$image" ] || die "not an image file: $image"
    image=$(listed_path "$image")

    # A VM that has the image open writes to it behind the mount's back, and
    # the mount writes behind the VM's: both would trust a stale view of the
    # same blocks. Refused whatever the mode, because a read-only mount of an
    # image that is being written reads a torn state.
    holders=$(lsof -t -- "$image" 2>/dev/null || true)
    if [ -n "$holders" ]; then
        say "refusing: $image is open by another process; stop it first:"
        for pid in $holders; do
            ps -o pid=,command= -p "$pid" >&2 || true
        done
        exit 1
    fi

    case "$2" in
    /Volumes/*) die "refusing a mountpoint under /Volumes: a mount that dies there blocks Finder, Spotlight and diskutil (testing/mounted-volume-testing.md); use a directory of your own" ;;
    esac
    mkdir -p "$2"
    mountpoint=$(cd "$2" && pwd -P)
    if is_mounted "$mountpoint"; then
        die "something is already mounted at $mountpoint"
    fi
    [ -z "$(ls -A "$mountpoint")" ] || die "the mountpoint is not empty: $mountpoint"
    state=$(state_file "$mountpoint")
    if [ -f "$state" ]; then
        old_pid=$(sed -n 1p "$state")
        if kill -0 "$old_pid" 2>/dev/null; then
            die "afsplus-mount (pid $old_pid) still serves $mountpoint; run: $0 unmount $mountpoint"
        fi
        rm -f "$state"
    fi

    ensure_tools
    say "checking $image"
    check_image "$image" || die "the checker does not find $image clean; not mounting it"

    mkdir -p "$state_dir"
    tmp=${TMPDIR:-/tmp}
    log=$(mktemp "${tmp%/}/afsplus-mount.XXXXXX")
    # nohup: the volume stays mounted after this shell closes. Stopping it is
    # always an unmount, never a kill.
    if [ -n "$mode" ]; then
        nohup "$bin/afsplus-mount" "$mode" "$image" "$mountpoint" >"$log" 2>&1 &
    else
        nohup "$bin/afsplus-mount" "$image" "$mountpoint" >"$log" 2>&1 &
    fi
    pid=$!
    printf '%s\n%s\n%s\n' "$pid" "$image" "$log" >"$state"

    waited=0
    until is_mounted "$mountpoint"; do
        if ! kill -0 "$pid" 2>/dev/null; then
            rm -f "$state"
            cat "$log" >&2
            die "afsplus-mount ended before the volume was mounted (log: $log)"
        fi
        waited=$((waited + 1))
        if [ "$waited" -gt 150 ]; then
            say "the volume did not appear within 30 seconds; stopping afsplus-mount"
            kill -TERM "$pid" 2>/dev/null || true
            cat "$log" >&2
            exit 1
        fi
        perl -e 'select(undef, undef, undef, 0.2)'
    done
    say "mounted ${mode:+read-only }$image at $mountpoint (afsplus-mount pid $pid, log $log)"
    say "unmount with: $0 unmount $mountpoint"
}

do_unmount() {
    force=
    if [ "${1:-}" = --force ]; then
        force=-f
        shift
    fi
    [ $# -eq 1 ] || usage
    mountpoint=$(listed_path "${1%/}")
    state=$(state_file "$mountpoint")
    pid=
    image=
    if [ -f "$state" ]; then
        pid=$(sed -n 1p "$state")
        image=$(sed -n 2p "$state")
        log=$(sed -n 3p "$state")
    fi

    if is_mounted "$mountpoint"; then
        # Plain umount refuses while a program has a file open or its working
        # directory on the volume; that refusal protects the program's data.
        if ! umount $force "$mountpoint"; then
            die "the volume is busy: close the programs using $mountpoint (a shell cd'd into it, Finder windows, an editor), then retry; --force unmounts anyway and those programs get I/O errors"
        fi
    else
        say "nothing is mounted at $mountpoint"
    fi

    if [ -n "$pid" ]; then
        # The driver commits its last state as the unmount ends; the image is
        # complete only once the process has gone.
        waited=0
        while kill -0 "$pid" 2>/dev/null; do
            waited=$((waited + 1))
            [ "$waited" -le 300 ] || die "afsplus-mount (pid $pid) is still running 60 seconds after the unmount; log: $log"
            perl -e 'select(undef, undef, undef, 0.2)'
        done
        rm -f "$state"
    fi
    if [ -n "$image" ]; then
        ensure_tools
        say "checking $image"
        check_image "$image" || die "the checker reports a problem in $image (log: $log)"
        say "unmounted; $image is clean"
    fi
}

command=${1:-}
[ $# -gt 0 ] && shift
case "$command" in
mount) do_mount "$@" ;;
unmount | umount) do_unmount "$@" ;;
*) usage ;;
esac
