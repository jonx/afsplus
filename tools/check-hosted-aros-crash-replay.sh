#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause

# Replay deterministic intent-log power-cut images through Hosted MacAROS.

set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
macaros_root=${MACAROS_ROOT:-"$repo_root/../Macaros"}
aros_build=${AROS_BUILD:-"$HOME/aros-build"}
aros_tree="$aros_build/bin/darwin-aarch64/AROS"
control="$macaros_root/graft/aros-ctl"
output=${AFSPLUS_HOSTED_REPLAY_OUTPUT:-"$repo_root/build/hosted-aros-crash-replay"}
work=$(mktemp -d "${TMPDIR:-/tmp}/afsplus-hosted-replay.XXXXXX")
package="$work/package"
fixtures="$work/fixtures"
result="$work/result"
aros_started=0
installed=0

handler="$aros_tree/L/afsplus-handler"
probe="$aros_tree/C/AFSPlusReplayProbe"
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
        echo "[hosted-replay] keeping failed work directory: $work" >&2
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
    expected=$2
    AROS_CTL_STARTUP_MODE=minimal \
    AROS_CTL_HOST_FOLDER="$phase_result" \
    AROS_CTL_STARTUP_EXTRA="Assign \"FDSK:\" \"SYS:DiskImages\"
C:Mount DEVS:DOSDrivers/AFSPLUS19 >MacRW:mount.out
C:AFSPlusReplayProbe $expected >MacRW:probe.out" \
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

require_executable "$control"
require_file "$aros_tree/Devs/fdsk.device"
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

mkdir "$result" "$result/cases"
cd "$repo_root"

echo "[hosted-replay] build a fresh qualified package"
AFSPLUS_AROS_PACKAGE_OUTPUT="$package" tools/package-aros-alpha0.sh

echo "[hosted-replay] generate deterministic power-cut images"
cargo run --quiet --release -p afsplus-check \
    --bin afsplus-crash-fixtures -- "$fixtures"
cp "$fixtures/manifest.tsv" "$fixtures/README.txt" "$result/"

installed=1
cp "$package/afsplus-handler" "$handler"
cp "$package/AFSPlusReplayProbe" "$probe"
cp "$package/AFSPLUS19" "$dosdriver"

tab=$(printf '\t')
while IFS="$tab" read -r fixture expected pending_before description; do
    [ "$fixture" != fixture ] || continue
    case_name=${fixture%.img}
    phase="$result/cases/$case_name"
    mkdir "$phase"
    cp "$fixtures/$fixture" "$image"

    echo "[hosted-replay] $case_name: expect $expected ($description)"
    cargo run --quiet --release -p afsplus-check --bin afsplus-check -- "$image" --json \
        >"$phase/check-before.json"
    grep -q '"clean":true' "$phase/check-before.json"
    grep -q "\"log_records_pending\":$pending_before" \
        "$phase/check-before.json"

    start_aros "$phase" "$expected"
    stop_aros "$phase"
    grep -q "^\[AFSPLUS-REPLAY\] PASS expected=$expected" "$phase/probe.out"

    cargo run --quiet --release -p afsplus-check --bin afsplus-check -- "$image" --json \
        >"$phase/check-after.json"
    grep -q '"clean":true' "$phase/check-after.json"
    grep -q '"log_records_pending":0' "$phase/check-after.json"
done <"$fixtures/manifest.tsv"

cp "$package/SHA256SUMS" "$result/package-SHA256SUMS"
(
    cd "$result"
    {
        find cases -type f -print
        printf '%s\n' README.txt manifest.tsv package-SHA256SUMS
    } | LC_ALL=C sort | xargs shasum -a 256 >SHA256SUMS
)

mkdir -p "$(dirname -- "$output")"
mv "$result" "$output"
echo "[hosted-replay] PASS: deterministic crash replay through Hosted MacAROS"
echo "[hosted-replay] result: $output"
