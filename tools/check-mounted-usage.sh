#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause

# What a person can actually do with a mounted AFS+ volume.
#
# The workspace suite proves the code does what it was written to do. This
# battery asks a different question, and the two are not interchangeable: it
# uses the volume the way somebody uses a disk, with the tools they already
# have, and reports what worked in the words they would use. Every line names
# an activity, never a call: "copy a folder into itself", not "setattr".
#
# It needs no hand. It builds the binaries, makes its own image, mounts it,
# works, unmounts, and says what happened. It never touches a volume it did
# not create.
#
#   tools/check-mounted-usage.sh                  run everything
#   AFSPLUS_USAGE_KEEP=1 tools/check-mounted-usage.sh   keep the image and mount
#
# Exit 0 when nothing unexpected failed. Exit 1 when something failed that was
# not already known to fail. Exit 66 when the battery could not run at all,
# which is not the same as the volume being broken.

set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
work=$(mktemp -d /tmp/afsplus-usage.XXXXXX)
image="$work/usage.img"
# One mountpoint per run. A previous run whose driver died leaves the name
# behind as a path that hangs rather than as a mount, and reusing it wedges the
# next run instead of reporting anything.
mountpoint=${AFSPLUS_USAGE_MOUNTPOINT:-/Volumes/AfsplusUsage-$$}
report="$work/diagnostics.txt"
size_mib=${AFSPLUS_USAGE_SIZE_MIB:-64}
keep=${AFSPLUS_USAGE_KEEP:-0}
mount_log="$work/mount.log"
mount_pid=

passed=0
failed=0
known=0
failures=""

# A failure a person can read, followed by the thread that owns the cause.
# Known failures keep running so the battery reports the whole picture rather
# than stopping at the first thing we have not fixed yet. A battery that skips
# what is broken teaches nothing.
known_failure() {
    case "$1" in
    modes) echo "thread 20, the POSIX mode is not stored, so any mode but the mount's own is refused" ;;
    times) echo "thread 20, a timestamp written with touch is accepted and discarded" ;;
    df-used) echo "not proven to be ours: the kernel's own statfs reports the right figures for this volume, so suspect df or the macFUSE backend before the driver" ;;
    *) echo "" ;;
    esac
}

say() { printf '%s\n' "$*"; }

result_pass() {
    passed=$((passed + 1))
    say "  ok    $1"
}

result_fail() {
    activity=$1
    detail=${2:-}
    owner=$(known_failure "${3:-}")
    if [ -n "$owner" ]; then
        known=$((known + 1))
        say "  known $activity"
        say "        known to fail: $owner"
        [ -z "$detail" ] || say "        $detail"
        return
    fi
    failed=$((failed + 1))
    failures="$failures
  $activity"
    say "  FAIL  $activity"
    [ -z "$detail" ] || say "        $detail"
}

# check <activity> [known-tag] -- <command...>
# The activity is what a person was doing. The command's own output is shown
# only when it fails, because a passing line should be readable at a glance.
check() {
    activity=$1
    shift
    tag=
    if [ "$1" != "--" ]; then
        tag=$1
        shift
    fi
    shift # the --
    if output=$("$@" 2>&1); then
        result_pass "$activity"
    else
        result_fail "$activity" "$(printf '%s' "$output" | head -3 | tr '\n' ' ')" "$tag"
    fi
}

# expect_equal <activity> <expected> <actual>
expect_equal() {
    if [ "$2" = "$3" ]; then
        result_pass "$1"
    else
        result_fail "$1" "expected $2, got $3" "${4:-}"
    fi
}

fatal() {
    say "cannot run the battery: $*"
    exit 66
}

# True when a path answers at all within a couple of seconds.
#
# A macFUSE mountpoint whose driver died is not in the mount table and yet
# every stat on it blocks for ever, so the ordinary tests, -d and -e, hang
# instead of returning false. Everything here that touches a mountpoint asks
# this first.
path_responds() {
    marker=$work/responds.$$
    rm -f "$marker"
    ( ls -d "$1" >/dev/null 2>&1; echo done > "$marker" ) &
    prober=$!
    tries=0
    while [ "$tries" -lt 20 ]; do
        [ -f "$marker" ] && { kill "$prober" 2>/dev/null; return 0; }
        sleep 0.1
        tries=$((tries + 1))
    done
    kill -9 "$prober" 2>/dev/null
    return 1
}

# Unmount and confirm it through the mount table.
#
# Never diskutil. It enumerates every volume before acting, so one wedged
# mountpoint anywhere on the machine makes every diskutil call block for ever,
# whatever volume you asked about; `|| true` does not rescue a command that
# never exits. This is what stopped this battery's first author, twice.
release_mount() {
    mount | grep -q " on $mountpoint " || return 0
    umount "$mountpoint" >/dev/null 2>&1 || umount -f "$mountpoint" >/dev/null 2>&1 || true
    tries=0
    while mount | grep -q " on $mountpoint "; do
        sleep 1
        tries=$((tries + 1))
        [ "$tries" -lt 15 ] || { say "        the volume would not unmount"; return 1; }
    done
    return 0
}

cleanup() {
    status=$?
    release_mount
    [ -z "$mount_pid" ] || wait "$mount_pid" 2>/dev/null || true
    if [ "$keep" = 1 ]; then
        say ""
        say "kept: image $image, diagnostics $report"
        exit $status
    fi
    [ ! -d "$work" ] || rm -r "$work"
    exit $status
}
trap cleanup EXIT HUP INT TERM

# ---------------------------------------------------------------- build

[ "$(uname -s)" = Darwin ] || fatal "this battery drives a macFUSE mount and needs macOS"
[ -d /Library/Frameworks/macFUSE.framework ] || fatal "macFUSE is not installed"

say "building the tools a person would use"
cd "$repo_root"
CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$repo_root/target}
export CARGO_TARGET_DIR
cargo build -q -p afsplus-tools --bin mkafsplus ||
    fatal "cannot build mkafsplus"
cargo build -q -p afsplus-fuse --features macfuse-mount --bin afsplus-mount ||
    fatal "cannot build afsplus-mount with macfuse-mount"
mkafsplus="$CARGO_TARGET_DIR/debug/mkafsplus"
afsplus_mount="$CARGO_TARGET_DIR/debug/afsplus-mount"

# ---------------------------------------------------------------- mount

say "making a ${size_mib} MiB volume and mounting it at $mountpoint"
"$mkafsplus" --size-mib "$size_mib" --label AfsplusUsage "$image" >/dev/null ||
    fatal "cannot format the image"

if mount | grep -q " on $mountpoint "; then
    fatal "$mountpoint is already mounted; this battery only uses a volume it made"
fi
if ! path_responds "$(dirname "$mountpoint")"; then
    fatal "$(dirname "$mountpoint") does not answer; a wedged mountpoint elsewhere is blocking it"
fi

"$afsplus_mount" --diagnostics="$report" "$image" "$mountpoint" >"$mount_log" 2>&1 &
mount_pid=$!

waited=0
while ! mount | grep -q " on $mountpoint "; do
    sleep 1
    waited=$((waited + 1))
    if [ "$waited" -gt 20 ]; then
        say "mount output:"
        sed 's/^/  /' "$mount_log"
        fatal "the volume did not appear at $mountpoint"
    fi
done
say "mounted"
say ""

# ---------------------------------------------------------------- using it

here=$mountpoint

say "making and naming things"
check "make a folder" -- mkdir "$here/Documents"
check "make a file with some text in it" -- sh -c "echo 'hello there' > '$here/Documents/note.txt'"
check "make a folder with an accent in its name" -- mkdir "$here/Documents/Éléments"
check "make a file with an emoji in its name" -- sh -c "echo x > '$here/Documents/🎉 party.txt'"
check "make a file with a very long name" -- sh -c "echo x > '$here/Documents/$(printf 'n%.0s' $(seq 1 200)).txt'"
check "make a dotfile" -- sh -c "echo x > '$here/Documents/.hidden'"
check "make a deeply nested tree" -- mkdir -p "$here/deep/a/b/c/d/e/f/g/h/i/j"
check "put a file at the bottom of a deep tree" -- sh -c "echo deep > '$here/deep/a/b/c/d/e/f/g/h/i/j/bottom.txt'"

say ""
say "reading back what was written"
written="the bytes I wrote are the bytes I read"
printf 'round trip payload\n' > "$work/payload.txt"
cp "$work/payload.txt" "$here/Documents/payload.txt" 2>/dev/null || true
if cmp -s "$work/payload.txt" "$here/Documents/payload.txt"; then
    result_pass "$written"
else
    result_fail "$written" "the file read back differs from the file written"
fi

say ""
say "writing in the ways a person writes"
check "append to a file" -- sh -c "echo 'second line' >> '$here/Documents/note.txt'"
expect_equal "an appended file has both lines" "2" \
    "$(wc -l < "$here/Documents/note.txt" | tr -d ' ')"
check "overwrite a file in place" -- sh -c "printf 'replaced\n' > '$here/Documents/note.txt'"
expect_equal "an overwritten file has only the new text" "replaced" \
    "$(cat "$here/Documents/note.txt")"
check "truncate a file to nothing" -- sh -c ": > '$here/Documents/note.txt'"
expect_equal "a truncated file is empty" "0" \
    "$(wc -c < "$here/Documents/note.txt" | tr -d ' ')"

say ""
say "a large file and its checksum"
dd if=/dev/urandom of="$work/big.bin" bs=1m count=8 >/dev/null 2>&1
check "copy an 8 MiB file onto the volume" -- cp "$work/big.bin" "$here/big.bin"
expect_equal "the large file survives the round trip" \
    "$(shasum -a 256 < "$work/big.bin" | cut -d' ' -f1)" \
    "$(shasum -a 256 < "$here/big.bin" 2>/dev/null | cut -d' ' -f1)"

say ""
say "many small files"
check "make two hundred small files" -- sh -c "
    mkdir -p '$here/many' &&
    i=0
    while [ \$i -lt 200 ]; do
        echo \$i > '$here/many/file-'\$i.txt || exit 1
        i=\$((i + 1))
    done"
expect_equal "all two hundred are listed" "200" \
    "$(ls "$here/many" 2>/dev/null | wc -l | tr -d ' ')"

say ""
say "copying, the way the Finder does it"
check "copy a folder recursively" -- cp -R "$here/Documents" "$here/Documents copy"
expect_equal "the copied folder has the same number of items" \
    "$(ls -A "$here/Documents" | wc -l | tr -d ' ')" \
    "$(ls -A "$here/Documents copy" 2>/dev/null | wc -l | tr -d ' ')"
check "duplicate a file into the same folder" -- cp "$here/Documents/payload.txt" "$here/Documents/payload copy.txt"
check "copy a folder in, preserving times and modes" -- cp -Rp "$here/Documents" "$here/Documents preserved"

say ""
say "moving and renaming"
check "rename a file" -- mv "$here/Documents/payload copy.txt" "$here/Documents/renamed.txt"
check "rename a folder" -- mv "$here/Documents/Éléments" "$here/Documents/Elements"
check "move a folder somewhere else" -- mv "$here/Documents/Elements" "$here/Elements"
check "move a file into another folder" -- mv "$here/Documents/renamed.txt" "$here/Elements/renamed.txt"

say ""
say "links"
check "make a symbolic link" -- ln -s "$here/Documents/payload.txt" "$here/Documents/link-to-payload"
check "make a hard link" -- ln "$here/Documents/payload.txt" "$here/Documents/hard-link"

say ""
say "the tools people already have"
check "search the volume with find" -- sh -c "find '$here' -name '*.txt' > /dev/null"
check "search inside files with grep" -- sh -c "grep -r 'round trip' '$here/Documents' > /dev/null"
check "measure a folder with du" -- sh -c "du -sh '$here/Documents' > /dev/null"
check "list a folder in detail" -- sh -c "ls -la '$here/Documents' > /dev/null"
check "make an archive with tar" -- tar -cf "$work/from-volume.tar" -C "$here" Documents
check "unpack an archive onto the volume" modes -- tar -xf "$work/from-volume.tar" -C "$here/many"
check "make a zip archive" -- sh -c "cd '$here' && zip -qr '$work/from-volume.zip' Documents"
check "copy a tree with rsync" modes -- rsync -a "$here/Documents/" "$here/rsynced/"
check "start a git repository on the volume" modes -- sh -c "cd '$here' && git init -q repo && cd repo && git -c user.email=a@b -c user.name=a commit -q --allow-empty -m first"

say ""
say "case, on a volume that does distinguish upper from lower"
check "make a file in one case" -- sh -c "echo one > '$here/CaseTest.txt'"
check "make another in the other case" -- sh -c "echo two > '$here/casetest.txt'"
expect_equal "the two are separate files" "one" "$(cat "$here/CaseTest.txt" 2>/dev/null)"

say ""
say "a file that is open while it changes underneath"
check "delete a file that is still open" -- sh -c "
    exec 3< '$here/Documents/hard-link'
    rm '$here/Documents/hard-link' || exit 1
    exec 3<&-"

say ""
say "a folder with a thousand entries"
check "make a thousand entries in one folder" -- sh -c "
    mkdir -p '$here/thousand' &&
    i=0
    while [ \$i -lt 1000 ]; do
        : > '$here/thousand/entry-'\$i || exit 1
        i=\$((i + 1))
    done"
expect_equal "all thousand are listed" "1000" \
    "$(ls "$here/thousand" 2>/dev/null | wc -l | tr -d ' ')"

say ""
say "two writers at once"
check "write two files at the same time" -- sh -c "
    ( dd if=/dev/zero of='$here/concurrent-a.bin' bs=64k count=16 2>/dev/null ) &
    first=\$!
    ( dd if=/dev/zero of='$here/concurrent-b.bin' bs=64k count=16 2>/dev/null ) &
    second=\$!
    wait \$first && wait \$second"

say ""
say "the space I freed comes back"
free_blocks() {
    df -k "$mountpoint" 2>/dev/null | awk 'NR == 2 { print $4 }'
}
# Twice, and the SECOND round is the one that must balance.
#
# Writing and deleting a large file the first time can cost a volume something
# one-off: the structures that track retired blocks grow to hold it and keep
# their own storage, which the next round reuses. Measuring one round called
# that a loss and reported a megabyte missing that was never missing. What
# matters, and what a person would notice, is whether doing the same thing
# again shrinks the volume every time.
round_cost() {
    before=$(free_blocks)
    dd if=/dev/zero of="$here/temporary.bin" bs=1m count=20 >/dev/null 2>&1 || true
    rm -f "$here/temporary.bin"
    sync
    sleep 1
    after=$(free_blocks)
    echo $((before - after))
}
first=$(round_cost)
second=$(round_cost)
say "        a first write-and-delete of 20 MiB costs ${first}K, a second costs ${second}K"
if [ -z "$first" ] || [ -z "$second" ]; then
    result_fail "the space I freed comes back" "the volume reported no free space figure"
elif [ "$second" -le 64 ]; then
    result_pass "the space I freed comes back"
else
    result_fail "the space I freed comes back" \
        "${second}K goes every time the same file is written and deleted"
fi

say ""
say "what the volume says about itself"
used=$(df -k "$mountpoint" 2>/dev/null | awk 'NR == 2 { print $3 }')
if [ "${used:-0}" -gt 0 ]; then
    result_pass "the volume reports space in use after writing to it"
else
    result_fail "the volume reports space in use after writing to it" \
        "df shows ${used:-nothing} used on a volume holding several megabytes" df-used
fi

# ------------------------------------------------- unmount and come back

say ""
say "unmounting and mounting again, with the content still there"
before_listing=$(ls -A "$here" | sort | tr '\n' ' ')
release_mount
wait "$mount_pid" 2>/dev/null || true
mount_pid=

"$afsplus_mount" --diagnostics="$report" "$image" "$mountpoint" >>"$mount_log" 2>&1 &
mount_pid=$!
waited=0
while ! mount | grep -q " on $mountpoint "; do
    sleep 1
    waited=$((waited + 1))
    [ "$waited" -le 20 ] || break
done

if mount | grep -q " on $mountpoint "; then
    result_pass "the volume mounts again after being unmounted"
    after_listing=$(ls -A "$here" | sort | tr '\n' ' ')
    expect_equal "everything written before the unmount is still there" \
        "$before_listing" "$after_listing"
    expect_equal "a file written before the unmount still reads back" \
        "$(shasum -a 256 < "$work/big.bin" | cut -d' ' -f1)" \
        "$(shasum -a 256 < "$here/big.bin" 2>/dev/null | cut -d' ' -f1)"
else
    result_fail "the volume mounts again after being unmounted" "it did not come back"
fi

# ---------------------------------------------------------------- filling it

# Deliberately NOT here. Filling a volume until it refuses is the one thing
# that has wedged this machine: a driver that cannot publish its staged work
# used to answer "busy" to everything afterwards, the mount process sat in
# uninterruptible wait where no signal reaches it, and diskutil then blocked on
# that mountpoint whatever volume you asked about. Two reboots. A battery must
# not be able to do that to the machine it runs on.
#
# The behaviour is covered where it cannot take anything down with it:
# crates/afsplus-vfs/tests/full_volume.rs fills a volume, requires that the
# delete which frees the space still works, and runs in a fifth of a second
# with no kernel involved.

# ---------------------------------------------------------------- report

say ""
say "----------------------------------------------------------------"
say "passed: $passed    failed: $failed    known to fail: $known"
if [ -n "$failures" ]; then
    say ""
    say "what a person could not do:$failures"
fi
if [ -s "$report" ]; then
    say ""
    say "what the driver said about itself:"
    sed 's/^/  /' "$report"
fi
say "----------------------------------------------------------------"

[ "$failed" -eq 0 ]
