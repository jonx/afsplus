#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause

# DOS semantics beyond the Alpha-0 slice through a real dos.library on Hosted
# MacAROS, then a dismount with a notification message never replied.

set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
macaros_root=${MACAROS_ROOT:-"$repo_root/../Macaros"}
aros_build=${AROS_BUILD:-"$HOME/aros-build"}
aros_tree="$aros_build/bin/darwin-aarch64/AROS"
control="$macaros_root/graft/aros-ctl"
output=${AFSPLUS_HOSTED_DOS_OUTPUT:-"$repo_root/build/hosted-aros-dos"}
work=$(mktemp -d "${TMPDIR:-/tmp}/afsplus-hosted-dos.XXXXXX")
package="$work/package"
result="$work/result"
aros_started=0
installed=0

handler="$aros_tree/L/afsplus-handler"
probe="$aros_tree/C/AFSPlusDosProbe"
dosdriver="$aros_tree/Devs/DOSDrivers/AFSPLUS19"
image="$aros_tree/DiskImages/Unit19"

cleanup() {
    status=$?
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
        echo "[hosted-dos] keeping failed work directory: $work" >&2
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
    "$repo_root/tools/check-aros-serial-log.sh" \
        "$phase_result/aros-window.log"
}

check_image() {
    cargo run --quiet --release -p afsplus-check --bin afsplus-check -- \
        "$image" --json >"$1"
    grep -q '"clean":true' "$1"
}

require_executable "$control"
require_executable "$repo_root/tools/check-aros-serial-log.sh"
require_file "$aros_tree/Devs/fdsk.device"
require_file "$aros_tree/C/Mount"
require_file "$aros_tree/C/Assign"
[ -d "$aros_tree/DiskImages" ] || {
    echo "Missing Hosted MacAROS DiskImages directory: $aros_tree" >&2
    exit 69
}
[ ! -e "$output" ] || {
    echo "Refusing to replace existing result: $output" >&2
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

mkdir "$result" "$result/dos" "$result/hold"
cd "$repo_root"

echo "[hosted-dos] build a fresh qualified package"
AFSPLUS_AROS_PACKAGE_OUTPUT="$package" tools/package-aros-alpha0.sh

installed=1
cp "$package/afsplus-handler" "$handler"
cp "$package/AFSPlusDosProbe" "$probe"
cp "$package/AFSPLUS19" "$dosdriver"
cp "$package/Unit19" "$image"

echo "[hosted-dos] DOS semantics phase"
start_aros "$result/dos" \
    'C:FailAt 21
Assign "FDSK:" "SYS:DiskImages"
C:Mount DEVS:DOSDrivers/AFSPLUS19 >MacRW:mount.out
C:AFSPlusDosProbe >MacRW:probe.out
C:List AFSPLUS19: ALL >MacRW:list-after.out
C:Mount AFSPLUS19: SHUTDOWN >MacRW:shutdown.out
If WARN
    C:Echo fail >MacRW:shutdown.status
Else
    C:Echo pass >MacRW:shutdown.status
EndIf'
stop_aros "$result/dos"
grep -q '^\[AFSPLUS-DOS\] PASS ' "$result/dos/probe.out"
grep '^\[AFSPLUS-DOS\] self-overlap ' "$result/dos/probe.out" \
    >"$result/record-self-overlap.txt"
[ "$(cat "$result/dos/shutdown.status")" = pass ]
if grep -q 'dosprobe' "$result/dos/list-after.out"; then
    echo "The probe left its drawer behind" >&2
    exit 1
fi
check_image "$result/check-after-dos.json"

echo "[hosted-dos] dismount with an unreplied notification"
start_aros "$result/hold" \
    'C:FailAt 21
Assign "FDSK:" "SYS:DiskImages"
C:Mount DEVS:DOSDrivers/AFSPLUS19 >MacRW:mount.out
C:AFSPlusDosProbe HOLD >MacRW:hold.out
C:Mount AFSPLUS19: SHUTDOWN >MacRW:shutdown.out
If WARN
    C:Echo fail >MacRW:shutdown.status
Else
    C:Echo pass >MacRW:shutdown.status
EndIf
C:Assign AFSPLUS19: DISMOUNT >MacRW:dismount.out
If WARN
    C:Echo fail >MacRW:dismount.status
Else
    C:Echo pass >MacRW:dismount.status
EndIf
C:Echo alive >MacRW:after-dismount.status'
stop_aros "$result/hold"
grep -q '^\[AFSPLUS-DOS\] HOLD ' "$result/hold/hold.out"
[ "$(cat "$result/hold/shutdown.status")" = pass ]
[ "$(cat "$result/hold/dismount.status")" = pass ]
[ "$(cat "$result/hold/after-dismount.status")" = alive ]
check_image "$result/check-after-hold.json"

printf '%s\n' none >"$result/guest-failure-requester.txt"
cp "$package/SHA256SUMS" "$result/package-SHA256SUMS"
(
    cd "$result"
    shasum -a 256 check-after-dos.json check-after-hold.json \
        record-self-overlap.txt guest-failure-requester.txt >SHA256SUMS
)

mkdir -p "$(dirname -- "$output")"
mv "$result" "$output"
echo "[hosted-dos] PASS: $output"
