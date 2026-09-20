#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause

# S2 on Hosted MacAROS: AROS boots from an AFS+ partition. The system is copied
# onto an AFS+ file system, wrapped in a GPT disk image as an AROS partition of
# DosType AFS+ (ADR-122), and served by hostdisk.device, whose unit pattern
# comes from the kernel argument hostdisk= (native/aros/aros-patches). The
# AFS+ handler and hostdisk are boot modules; bootdevice= selects the
# partition. The Startup-Sequence that runs is the one on AFS+, and it says so.
# The control boot, without the AFS+ handler module, falls back to the host
# volume. Afterwards the partition is taken out and checked.
#
# The AROS tree needs hostdisk.device built with the patch, partition.library
# (kernel-partition), and the desktop set of tools/check-hosted-aros-s1b.sh.
# The boot scan reads the GPT through partition.library, so it is a boot
# module too.
#
# The boot mount is made before the handler can be given a clock, so it starts
# SYNC and asks for timer.device again while it serves packets. The gate reads
# the policy SYS: runs under a few seconds into the boot and requires
# "delayed": a system volume that commits every change separately pays a
# device flush per operation on Native.
#
#   AFSPLUS_S2_OUTPUT   where the result goes (default build/hosted-aros-s2)
#   AFSPLUS_S2_TIMEOUT  seconds a boot may take (default 180)
#   AFSPLUS_S2_COMMIT_CONTROL=1  negative control: build the handler with SYNC
#                       as its default policy and without the retry. SYS: then
#                       runs SYNC and the gate must fail here. Disabling the
#                       retry alone proves nothing: timer.device opens at the
#                       boot mount, and the volume delays at once.

set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
macaros_root=${MACAROS_ROOT:-"$repo_root/../Macaros"}
aros_build=${AROS_BUILD:-"$HOME/aros-build"}
aros_tree="$aros_build/bin/darwin-aarch64/AROS"
boot_conf="$aros_tree/boot/darwin/AROSBootstrap.conf"
control="$macaros_root/graft/aros-ctl"
timeout=${AFSPLUS_S2_TIMEOUT:-180}
commit_control=${AFSPLUS_S2_COMMIT_CONTROL:-0}
output=${AFSPLUS_S2_OUTPUT:-"$repo_root/build/hosted-aros-s2"}
work=$(mktemp -d "${TMPDIR:-/tmp}/afsplus-s2.XXXXXX")
package="$work/package"
source_tree="$work/system"
result="$work/result"
aros_started=0
installed=0

handler="$aros_tree/L/afsplus-handler"
hostdisk="$aros_tree/Devs/hostdisk.device"
handler_line="module $handler"
hostdisk_line="module $hostdisk"
partition="$aros_tree/Libs/partition.library"
partition_line="module $partition"

remove_module_lines() {
    [ -f "$boot_conf" ] || return 0
    grep -vxF -e "$handler_line" -e "$hostdisk_line" -e "$partition_line" \
        "$boot_conf" | grep -v '^arguments .*hostdisk=' >"$work/conf" || true
    cat "$work/conf" >"$boot_conf"
}

cleanup() {
    status=$?
    if [ "$aros_started" = 1 ]; then
        "$control" stop >/dev/null 2>&1 || true
    fi
    remove_module_lines
    if [ "$installed" = 1 ] && [ -e "$handler" ]; then
        unlink "$handler"
    fi
    if [ "$status" -ne 0 ] && [ "${AFSPLUS_KEEP_HOSTED_FAILURE:-0}" = 1 ]; then
        echo "[hosted-s2] keeping failed work directory: $work" >&2
        return
    fi
    [ ! -d "$work" ] || rm -r "$work"
}
trap cleanup EXIT HUP INT TERM

[ -x "$control" ] || { echo "Missing $control" >&2; exit 69; }
[ -f "$boot_conf" ] || { echo "Missing $boot_conf" >&2; exit 69; }
[ -f "$hostdisk" ] || {
    echo "Missing $hostdisk; build workbench-devs-hostdisk with native/aros/aros-patches" >&2
    exit 69
}
for path in Libs/partition.library Prefs/Presets/Themes Prefs/Locale Libs/codesets.library System/Wanderer/Wanderer; do
    [ -e "$aros_tree/$path" ] || {
        echo "Missing $aros_tree/$path; see tools/check-hosted-aros-s1b.sh" >&2
        exit 69
    }
done
[ ! -e "$output" ] || {
    echo "Refusing to replace existing result: $output" >&2
    exit 73
}
[ ! -e "$handler" ] || {
    echo "Refusing to replace Hosted MacAROS artifact: $handler" >&2
    exit 73
}
if grep -qxF -e "$handler_line" -e "$hostdisk_line" -e "$partition_line" "$boot_conf"; then
    echo "The boot module list already names the handler, hostdisk or partition.library" >&2
    exit 73
fi
if ! "$control" status | grep -q '^state=stopped$'; then
    echo "Refusing to disturb a running Hosted MacAROS instance" >&2
    exit 75
fi

mkdir "$result"
cd "$repo_root"

if [ "$commit_control" = 1 ]; then
    echo "[hosted-s2] negative control: the handler never retries timer.device"
    AFSPLUS_AROS_HANDLER_CFLAGS="${AFSPLUS_AROS_HANDLER_CFLAGS:-} -DAFSPLUS_AROS_COMMIT_RETRY=0 -DAFSPLUS_CONTROL_COMMIT_DEFAULT=0"
    export AFSPLUS_AROS_HANDLER_CFLAGS
fi

echo "[hosted-s2] build a fresh qualified package"
AFSPLUS_AROS_PACKAGE_OUTPUT="$package" tools/package-aros-alpha0.sh
(cd "$package" && shasum -a 256 -c SHA256SUMS >/dev/null)

echo "[hosted-s2] copy the system onto an AFS+ partition"
mkdir "$source_tree"
for entry in "$aros_tree"/*; do
    case "${entry##*/}" in
        Developer|DiskImages|boot|S) ;;
        *) cp -R "$entry" "$source_tree/" ;;
    esac
done
# AROS.boot comes along: dos.library boots only from a volume whose AROS.boot
# names its CPU (rom/dos/isbootable.c).
[ -f "$source_tree/AROS.boot" ]
mkdir "$source_tree/S"
cp native/aros/tests/s2-startup-sequence "$source_tree/S/Startup-Sequence"
cp native/aros/tests/s1b-proof "$source_tree/S/AFSPlus-Proof"
cp native/aros/tests/s2-tour "$source_tree/S/AFSPlus-Tour"
cp "$package/AFSPlusTour" "$source_tree/C/AFSPlusTour"
cp "$package/AFSPlusInfo" "$source_tree/C/AFSPlusInfo"
cp "$package/afsplus-handler" "$source_tree/L/afsplus-handler"
printf '%s' afsplus-s2 >"$source_tree/s2-origin"
cargo run --quiet --release -p afsplus-tools --bin mkafsplus -- \
    --profile workstation --size-mib 128 --label AFSPlusS2 \
    --case-insensitive "$work/system.afsp" >/dev/null
cargo run --quiet --release -p afsplus-core --bin afsplus-populate -- \
    "$work/system.afsp" "$source_tree" >"$result/populate.txt"
cargo run --quiet --release -p afsplus-check --bin afsplus-check -- \
    "$work/system.afsp" --json >"$result/check-before.json"
grep -q '"clean":true' "$result/check-before.json"
cargo run --quiet --release -p afsplus-tools --bin afsplus-disk -- \
    wrap --bootpri 10 --name AFSSYS "$work/disk0.img" "$work/system.afsp" \
    >"$result/wrap.txt"
rm "$work/system.afsp"

installed=1
cp "$package/afsplus-handler" "$handler"

# One boot from whatever the boot scan finds; the done file says which
# Startup-Sequence ran.
boot() {
    run=$1
    mkdir "$result/$run"
    AROS_HOST_ARGS="hostdisk=$work/disk%ld.img bootdevice=AFSSYS" \
    AROS_CTL_STARTUP_MODE=minimal \
    AROS_CTL_HOST_FOLDER="$result/$run" \
    AROS_CTL_STARTUP_EXTRA='C:Echo host >MacRW:done' \
        "$control" run >/dev/null
    aros_started=1
    waited=0
    while [ ! -f "$result/$run/done" ]; do
        [ "$waited" -lt "$timeout" ] || {
            echo "[hosted-s2] $run: no end of the boot after $timeout seconds" >&2
            "$control" shot "$result/$run/timeout.png" >/dev/null 2>&1 || true
            exit 1
        }
        sleep 2
        waited=$((waited + 2))
    done
    sleep 3
    "$control" shot "$result/$run/desktop.png" >/dev/null || true
    "$control" tasks >"$result/$run/tasks.out" || true
    "$control" stop >/dev/null 2>&1 || true
    aros_started=0
    cp /tmp/aros-window.log "$result/$run/aros-window.log"
}

echo "[hosted-s2] boot from the AFS+ partition"
printf '%s\n%s\n%s\n' "$partition_line" "$hostdisk_line" "$handler_line" >>"$boot_conf"
boot s2
[ "$(cat "$result/s2/done")" = afsplus ] || {
    echo "[hosted-s2] the boot did not run the AFS+ Startup-Sequence: $(cat "$result/s2/done")" >&2
    exit 1
}
grep -q '^{"schema":"afsplus-handler-info"' "$result/s2/s2-info-sys.json"
grep -q '"label":"AFSPlusS2"' "$result/s2/s2-info-sys.json"
grep -q 'AFS+' "$result/s2/s2-info.out"
[ "$(cat "$result/s2/s2-origin")" = afsplus-s2 ]
# The tour of the v2 interface, run from the boot volume on itself: no step
# may fail, whatever steps it grows, the summary must pass, and these four
# must be there.
if grep -q '^\[AFSPLUS-TOUR\] .*FAIL$' "$result/s2/s2-tour.out" \
    || ! tail -n 1 "$result/s2/s2-tour.out" | grep -qx '\[AFSPLUS-TOUR\] PASS'; then
    echo "[hosted-s2] the tour failed:" >&2
    cat "$result/s2/s2-tour.out" >&2
    exit 1
fi
for tour_step in volume clone watch attribute; do
    grep -qx "\[AFSPLUS-TOUR\] $tour_step PASS" "$result/s2/s2-tour.out" || {
        echo "[hosted-s2] the tour failed at $tour_step:" >&2
        cat "$result/s2/s2-tour.out" >&2
        exit 1
    }
done

# The boot volume must group its changes: it is the one mount nothing can
# give a Control string, and the one that pays most for committing each
# change on its own.
commit_line=$(cat "$result/s2/s2-commit-sys.out" 2>/dev/null || true)
echo "[hosted-s2] SYS: $commit_line"
case "$commit_line" in
    "commit delayed "[1-9]*) ;;
    *)
        echo "[hosted-s2] SYS: does not run delayed commit: $commit_line" >&2
        exit 1
        ;;
esac

echo "[hosted-s2] the same boot without the AFS+ handler module"
remove_module_lines
printf '%s\n%s\n' "$partition_line" "$hostdisk_line" >>"$boot_conf"
boot control
remove_module_lines
[ "$(cat "$result/control/done")" = host ] || {
    echo "[hosted-s2] the control boot ran $(cat "$result/control/done")" >&2
    exit 1
}

echo "[hosted-s2] structural check of the AFS+ partition after the boot"
cargo run --quiet --release -p afsplus-tools --bin afsplus-disk -- \
    extract "$work/disk0.img" "$work/after.afsp" >/dev/null
cargo run --quiet --release -p afsplus-check --bin afsplus-check -- \
    "$work/after.afsp" --json >"$result/check-after.json"
grep -q '"clean":true' "$result/check-after.json"

mkdir -p "$(dirname -- "$output")"
mv "$result" "$output"
echo "[hosted-s2] PASS: AROS booted from an AFS+ partition"
echo "[hosted-s2] result: $output"
