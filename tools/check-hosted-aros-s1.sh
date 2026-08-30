#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause

# Hosted post-bootstrap SYS: pivot onto a manifested AFS+ system subset.

set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
macaros_root=${MACAROS_ROOT:-"$repo_root/../Macaros"}
aros_build=${AROS_BUILD:-"$HOME/aros-build"}
aros_tree="$aros_build/bin/darwin-aarch64/AROS"
control="$macaros_root/graft/aros-ctl"
output=${AFSPLUS_HOSTED_S1_OUTPUT:-"$repo_root/build/hosted-aros-s1"}
work=$(mktemp -d "${TMPDIR:-/tmp}/afsplus-hosted-s1.XXXXXX")
package="$work/package"
s1_image="$work/s1-image"
result="$work/result"
aros_started=0
installed=0

handler="$aros_tree/L/afsplus-handler"
pivot="$aros_tree/C/AFSPlusS1Pivot"
dosdriver="$aros_tree/Devs/DOSDrivers/AFSPLUS19"
image="$aros_tree/DiskImages/Unit19"

cleanup() {
    status=$?
    if [ "$aros_started" = 1 ]; then
        "$control" stop >/dev/null 2>&1 || true
    fi
    if [ "$installed" = 1 ]; then
        [ ! -e "$handler" ] || unlink "$handler"
        [ ! -e "$pivot" ] || unlink "$pivot"
        [ ! -e "$dosdriver" ] || unlink "$dosdriver"
        [ ! -e "$image" ] || unlink "$image"
    fi
    if [ "$status" -ne 0 ] && [ "${AFSPLUS_KEEP_HOSTED_FAILURE:-0}" = 1 ]; then
        echo "[hosted-s1] keeping failed work directory: $work" >&2
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

stop_aros() {
    "$control" status >"$result/status.txt" || true
    "$control" stop >/dev/null 2>&1 || true
    aros_started=0
    cp /tmp/aros-window.log "$result/aros-window.log"
    if grep -Eq 'AFSPLUS.*failed|Trap signal|ALERT' "$result/aros-window.log"; then
        echo "Hosted AROS reported an AFS+ failure or crash" >&2
        grep -En 'AFSPLUS.*failed|Trap signal|ALERT' "$result/aros-window.log" >&2
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
for target in "$handler" "$pivot" "$dosdriver" "$image"; do
    [ ! -e "$target" ] || {
        echo "Refusing to replace Hosted MacAROS artifact: $target" >&2
        exit 73
    }
done
if ! "$control" status | grep -q '^state=stopped$'; then
    echo "Refusing to disturb a running Hosted MacAROS instance" >&2
    exit 75
fi

mkdir "$result"
cd "$repo_root"

echo "[hosted-s1] build handler and target probes"
AFSPLUS_AROS_PACKAGE_OUTPUT="$package" tools/package-aros-alpha0.sh
echo "[hosted-s1] build manifested AFS+ system subset"
AFSPLUS_AROS_PACKAGE="$package" AFSPLUS_AROS_S1_OUTPUT="$s1_image" \
    tools/build-aros-s1-image.sh

installed=1
cp "$s1_image/afsplus-handler" "$handler"
cp "$package/AFSPlusS1Pivot" "$pivot"
cp "$s1_image/AFSPLUS19-S1" "$dosdriver"
cp "$s1_image/Unit19.s1" "$image"

echo "[hosted-s1] boot from the host tree, then pivot SYS: onto AFS+"
AROS_CTL_STARTUP_MODE=minimal \
AROS_CTL_HOST_FOLDER="$result" \
AROS_CTL_STARTUP_EXTRA='Assign "FDSK:" "SYS:DiskImages"
C:Mount DEVS:DOSDrivers/AFSPLUS19 >MacRW:s1-mount.out
C:AFSPlusS1Pivot >MacRW:s1-pivot.out' \
    "$control" run >/dev/null
aros_started=1
"$control" wait 12 >/dev/null
stop_aros

grep -q '^\[AFSPLUS-S1\] PASS ' "$result/s1-probe.out"
grep -q '^\[AFSPLUS-S1-PIVOT\] PASS' "$result/s1-pivot.out"
[ "$(cat "$result/s1-runtime")" = afsplus-s1 ]
[ -s "$result/s1-version.out" ]
grep -q 'S1-Sequence' "$result/s1-list.out"

cargo run --quiet --release -p afsplus-check --bin afsplus-check -- \
    "$image" --json >"$result/check-after.json"
grep -q '"clean":true' "$result/check-after.json"
grep -q '"log_records_pending":0' "$result/check-after.json"

cp "$image" "$result/Unit19.s1.final"
cp "$s1_image/content-SHA256SUMS" "$result/"
cp "$s1_image/check-before.json" "$result/"
cp "$package/SHA256SUMS" "$result/package-SHA256SUMS"
git -C "$macaros_root" rev-parse HEAD >"$result/macaros-commit.txt"
git -C "$macaros_root" status --short >"$result/macaros-status.txt"
(
    cd "$result"
    shasum -a 256 Unit19.s1.final check-before.json check-after.json \
        content-SHA256SUMS package-SHA256SUMS s1-probe.out s1-runtime \
        s1-version.out s1-list.out s1-pivot.out macaros-commit.txt \
        macaros-status.txt >SHA256SUMS
)

mkdir -p "$(dirname -- "$output")"
mv "$result" "$output"
echo "[hosted-s1] PASS: post-bootstrap SYS: pivot onto AFS+"
echo "[hosted-s1] result: $output"
