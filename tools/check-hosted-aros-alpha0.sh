#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause

# Bidirectional same-image gate: Hosted MacAROS -> host mount -> Hosted MacAROS.

set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
macaros_root=${MACAROS_ROOT:-"$repo_root/../Macaros"}
aros_build=${AROS_BUILD:-"$HOME/aros-build"}
aros_tree="$aros_build/bin/darwin-aarch64/AROS"
control="$macaros_root/graft/aros-ctl"
output=${AFSPLUS_HOSTED_RESULT_OUTPUT:-"$repo_root/build/hosted-aros-alpha0"}
work=$(mktemp -d "${TMPDIR:-/tmp}/afsplus-hosted-alpha0.XXXXXX")
package="$work/package"
result="$work/result"
host_mount="/Volumes/AFSPlusSameImage-$$"
host_mount_pid=
aros_started=0
installed=0

handler="$aros_tree/L/afsplus-handler"
probe="$aros_tree/C/AFSPlusAlpha0Probe"
dosdriver="$aros_tree/Devs/DOSDrivers/AFSPLUS19"
image="$aros_tree/DiskImages/Unit19"

cleanup() {
    status=$?
    if [ -n "$host_mount_pid" ] && kill -0 "$host_mount_pid" 2>/dev/null; then
        diskutil unmount "$host_mount" >/dev/null 2>&1 || true
        kill "$host_mount_pid" 2>/dev/null || true
        wait "$host_mount_pid" 2>/dev/null || true
    fi
    if [ "$aros_started" = 1 ]; then
        "$control" stop >/dev/null 2>&1 || true
    fi
    if [ "$installed" = 1 ]; then
        [ ! -e "$handler" ] || unlink "$handler"
        [ ! -e "$probe" ] || unlink "$probe"
        [ ! -e "$dosdriver" ] || unlink "$dosdriver"
        [ ! -e "$image" ] || unlink "$image"
    fi
    if [ "$status" -ne 0 ] && [ "${AFSPLUS_KEEP_HOSTED_FAILURE:-0}" = 1 ]; then
        echo "[hosted-alpha0] keeping failed work directory: $work" >&2
        return
    fi
    [ ! -d "$work" ] || rm -r "$work"
}
trap cleanup EXIT HUP INT TERM

require_file() {
    [ -f "$1" ] || {
        echo "Missing required file: $1" >&2
        exit 66
    }
}

require_executable() {
    [ -x "$1" ] || {
        echo "Missing required executable: $1" >&2
        exit 69
    }
}

start_aros() {
    phase_result=$1
    startup=$2
    AROS_CTL_STARTUP_MODE=minimal \
    AROS_CTL_HOST_FOLDER="$phase_result" \
    AROS_CTL_STARTUP_EXTRA="$startup" \
        "$control" run >/dev/null
    aros_started=1
    "$control" wait 8 >/dev/null
}

stop_aros() {
    phase_result=$1
    "$control" status >"$phase_result/status.txt" || true
    "$control" stop >/dev/null 2>&1 || true
    aros_started=0
    cp /tmp/aros-window.log "$phase_result/aros-window.log"
    if grep -Eq 'AFSPLUS.*failed|Trap signal|ALERT' \
        "$phase_result/aros-window.log"; then
        echo "Hosted AROS reported an AFS+ failure or crash" >&2
        grep -En 'AFSPLUS.*failed|Trap signal|ALERT' \
            "$phase_result/aros-window.log" >&2
        exit 1
    fi
}

wait_for_host_mount() {
    tries=0
    while [ "$tries" -lt 200 ]; do
        if [ -f "$host_mount/alpha0.from-aros" ]; then
            return 0
        fi
        if ! kill -0 "$host_mount_pid" 2>/dev/null; then
            wait "$host_mount_pid" || true
            cat "$result/host-mount.log" >&2
            return 1
        fi
        sleep 0.05
        tries=$((tries + 1))
    done
    echo "Timed out waiting for the host AFS+ mount" >&2
    return 1
}

require_executable "$control"
require_file "$aros_tree/Devs/fdsk.device"
require_file "$aros_tree/C/Mount"
require_file "$aros_tree/C/Assign"
command -v strings >/dev/null 2>&1 || {
    echo "Missing required host command: strings" >&2
    exit 69
}
if ! strings "$aros_tree/C/Mount" | grep -q 'SHUTDOWN/S'; then
    echo "Hosted AROS C:Mount is too old: SHUTDOWN support is required" >&2
    exit 69
fi
[ -d "$aros_tree/DiskImages" ] || {
    echo "Missing Hosted MacAROS DiskImages directory: $aros_tree" >&2
    exit 69
}
[ ! -e "$output" ] || {
    echo "Refusing to replace existing result: $output" >&2
    exit 73
}
[ ! -e "$host_mount" ] || {
    echo "Refusing existing host mountpoint: $host_mount" >&2
    exit 73
}
for target in "$handler" "$probe" "$dosdriver" "$image"; do
    [ ! -e "$target" ] || {
        echo "Refusing to replace Hosted MacAROS artifact: $target" >&2
        exit 73
    }
done
if ! "$control" status | grep -q '^state=stopped$'; then
    echo "Refusing to disturb a running Hosted MacAROS instance" >&2
    exit 75
fi

mkdir "$result" "$result/target" "$result/return"
cd "$repo_root"

echo "[hosted-alpha0] build a fresh qualified package"
AFSPLUS_AROS_PACKAGE_OUTPUT="$package" tools/package-aros-alpha0.sh

installed=1
cp "$package/afsplus-handler" "$handler"
cp "$package/AFSPlusAlpha0Probe" "$probe"
cp "$package/AFSPLUS19" "$dosdriver"
cp "$package/Unit19" "$image"

echo "[hosted-alpha0] target operation phase"
start_aros "$result/target" \
    'Assign "FDSK:" "SYS:DiskImages"
C:Mount DEVS:DOSDrivers/AFSPLUS19 >MacRW:mount.out
C:List AFSPLUS19: ALL >MacRW:list-before.out
C:AFSPlusAlpha0Probe >MacRW:probe.out
C:List AFSPLUS19: ALL >MacRW:list-after.out
C:Copy AFSPLUS19:alpha0.from-aros MacRW:alpha0.from-aros >MacRW:copy.out'
stop_aros "$result/target"
grep -q '^\[AFSPLUS-ALPHA0\] PASS ' "$result/target/probe.out"
[ "$(cat "$result/target/alpha0.from-aros")" = hello ]
cargo run --quiet --release -p afsplus-check --bin afsplus-check -- "$image" --json \
    >"$result/check-after-target.json"
grep -q '"clean":true' "$result/check-after-target.json"

echo "[hosted-alpha0] host same-image phase"
cargo build -p afsplus-fuse --features macfuse-mount --bins
./target/debug/afsplus-mount "$image" "$host_mount" \
    >"$result/host-mount.log" 2>&1 &
host_mount_pid=$!
wait_for_host_mount
./target/debug/afsplus-mounted-alpha0 "$host_mount" \
    >"$result/host-probe.out"
grep -q '^\[AFSPLUS-HOST-ALPHA0\] PASS ' "$result/host-probe.out"
[ "$(cat "$host_mount/alpha0.from-host")" = host ]
diskutil unmount "$host_mount" >"$result/host-unmount.out"
wait "$host_mount_pid"
host_mount_pid=
cargo run --quiet --release -p afsplus-check --bin afsplus-check -- "$image" --json \
    >"$result/check-after-host.json"
grep -q '"clean":true' "$result/check-after-host.json"

echo "[hosted-alpha0] target return-read phase"
start_aros "$result/return" \
    'C:FailAt 21
Assign "FDSK:" "SYS:DiskImages"
C:Mount DEVS:DOSDrivers/AFSPLUS19 >MacRW:mount.out
C:List AFSPLUS19: ALL >MacRW:list.out
C:Copy AFSPLUS19:alpha0.from-host MacRW:alpha0.from-host >MacRW:copy-host.out
C:Copy AFSPLUS19:alpha0.from-aros MacRW:alpha0.from-aros >MacRW:copy-aros.out
C:Mount AFSPLUS19: SHUTDOWN >MacRW:shutdown-1.out
If WARN
    C:Echo fail >MacRW:shutdown-1.status
Else
    C:Echo pass >MacRW:shutdown-1.status
EndIf
C:List AFSPLUS19: >MacRW:restart-1.out
If WARN
    C:Echo fail >MacRW:restart-1.status
Else
    C:Echo pass >MacRW:restart-1.status
EndIf
C:Copy AFSPLUS19:alpha0.from-host MacRW:alpha0.after-restart-1 >MacRW:copy-restart-1.out
C:Mount AFSPLUS19: SHUTDOWN >MacRW:shutdown-2.out
If WARN
    C:Echo fail >MacRW:shutdown-2.status
Else
    C:Echo pass >MacRW:shutdown-2.status
EndIf
C:List AFSPLUS19: >MacRW:restart-2.out
If WARN
    C:Echo fail >MacRW:restart-2.status
Else
    C:Echo pass >MacRW:restart-2.status
EndIf
C:Copy AFSPLUS19:alpha0.from-aros MacRW:alpha0.after-restart-2 >MacRW:copy-restart-2.out
C:Mount AFSPLUS19: SHUTDOWN >MacRW:shutdown-final.out
If WARN
    C:Echo fail >MacRW:shutdown-final.status
Else
    C:Echo pass >MacRW:shutdown-final.status
EndIf
C:Assign AFSPLUS19: DISMOUNT >MacRW:dismount-final.out
If WARN
    C:Echo fail >MacRW:dismount-final.status
Else
    C:Echo pass >MacRW:dismount-final.status
EndIf'
stop_aros "$result/return"
[ "$(cat "$result/return/alpha0.from-host")" = host ]
[ "$(cat "$result/return/alpha0.from-aros")" = hello ]
[ "$(cat "$result/return/shutdown-1.status")" = pass ]
[ "$(cat "$result/return/restart-1.status")" = pass ]
[ "$(cat "$result/return/alpha0.after-restart-1")" = host ]
[ "$(cat "$result/return/shutdown-2.status")" = pass ]
[ "$(cat "$result/return/restart-2.status")" = pass ]
[ "$(cat "$result/return/alpha0.after-restart-2")" = hello ]
[ "$(cat "$result/return/shutdown-final.status")" = pass ]
[ "$(cat "$result/return/dismount-final.status")" = pass ]
if grep -Eq '^\._alpha0\.' "$result/return/list.out"; then
    echo "AppleDouble sidecars leaked into the final fixture" >&2
    exit 1
fi
cargo run --quiet --release -p afsplus-check --bin afsplus-check -- "$image" --json \
    >"$result/check-final.json"
grep -q '"clean":true' "$result/check-final.json"

cp "$image" "$result/Unit19.final"
cp "$package/SHA256SUMS" "$result/package-SHA256SUMS"
(
    cd "$result"
    shasum -a 256 Unit19.final check-after-target.json \
        check-after-host.json check-final.json >SHA256SUMS
)

mkdir -p "$(dirname -- "$output")"
mv "$result" "$output"
echo "[hosted-alpha0] PASS: bidirectional same-image S0"
echo "[hosted-alpha0] result: $output"
