#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause

# The C13 benchmark runner on Hosted MacAROS: the small-file development-tree
# workload of AFSPlusBench on an AFS+ volume and on a Fast File System volume
# of the same size, in one boot, with the same seed. Before: the package and
# the AFS+ image are verified against the package manifest and the image
# checker. After: the AFS+ image is checked again. The result bundle is
# results.json in the format of testing/benchmark-contract.md section 7.
#
#   AFSPLUS_BENCH_SEED     workload seed (default 1)
#   AFSPLUS_BENCH_TREES    trees of 256 files each phase runs over (default 10)
#   AFSPLUS_BENCH_OUTPUT   where the bundle goes (default build/bench-hosted-aros)
#   AFSPLUS_BENCH_TIMEOUT  seconds the boot may take (default 900)

set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
macaros_root=${MACAROS_ROOT:-"$repo_root/../Macaros"}
aros_build=${AROS_BUILD:-"$HOME/aros-build"}
aros_tree="$aros_build/bin/darwin-aarch64/AROS"
control="$macaros_root/graft/aros-ctl"
seed=${AFSPLUS_BENCH_SEED:-1}
trees=${AFSPLUS_BENCH_TREES:-10}
timeout=${AFSPLUS_BENCH_TIMEOUT:-900}
output=${AFSPLUS_BENCH_OUTPUT:-"$repo_root/build/bench-hosted-aros"}
work=$(mktemp -d "${TMPDIR:-/tmp}/afsplus-bench.XXXXXX")
package="$work/package"
result="$work/result"
aros_started=0
installed=0

handler="$aros_tree/L/afsplus-handler"
bench="$aros_tree/C/AFSPlusBench"
dosdriver="$aros_tree/Devs/DOSDrivers/AFSPLUS19"
basedriver="$aros_tree/Devs/DOSDrivers/BASE20"
image="$aros_tree/DiskImages/Unit19"
baseimage="$aros_tree/DiskImages/Unit20"

cleanup() {
    status=$?
    if [ "$aros_started" = 1 ]; then
        "$control" stop >/dev/null 2>&1 || true
    fi
    if [ "$installed" = 1 ]; then
        for target in "$handler" "$bench" "$dosdriver" "$basedriver" \
                "$image" "$baseimage"; do
            [ ! -e "$target" ] || unlink "$target"
        done
    fi
    if [ "$status" -ne 0 ] && [ "${AFSPLUS_KEEP_HOSTED_FAILURE:-0}" = 1 ]; then
        echo "[bench] keeping failed work directory: $work" >&2
        return
    fi
    [ ! -d "$work" ] || rm -r "$work"
}
trap cleanup EXIT HUP INT TERM

for number in "$seed" "$trees" "$timeout"; do
    case $number in
        ''|*[!0-9]*) echo "seed, trees and timeout must be decimal numbers" >&2; exit 64 ;;
    esac
done
[ -x "$control" ] || { echo "Missing $control" >&2; exit 69; }
for path in "$aros_tree/Devs/fdsk.device" "$aros_tree/L/afs-handler" \
        "$aros_tree/C/Mount"; do
    [ -f "$path" ] || {
        echo "Missing $path; run tools/prepare-hosted-aros.sh" >&2
        exit 69
    }
done
[ ! -e "$output" ] || {
    echo "Refusing to replace existing result: $output" >&2
    exit 73
}
for target in "$handler" "$bench" "$dosdriver" "$basedriver" "$image" \
        "$baseimage"; do
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

echo "[bench] build a fresh qualified package"
AFSPLUS_AROS_PACKAGE_OUTPUT="$package" tools/package-aros-alpha0.sh

echo "[bench] verify the package against its manifest"
(cd "$package" && shasum -a 256 -c SHA256SUMS >/dev/null)
grep -q '"clean":true' "$package/check-before.json"

installed=1
cp "$package/afsplus-handler" "$handler"
cp "$package/AFSPlusBench" "$bench"
cp "$package/AFSPLUS19" "$dosdriver"
cp "$package/BASE20" "$basedriver"
cp "$package/Unit19" "$image"
# The baseline image has the size of the AFS+ one and starts as zeros; the
# runner formats it through dos.library in the boot that measures it.
dd if=/dev/zero of="$baseimage" bs=1048576 count=64 2>/dev/null
shasum -a 256 "$image" "$baseimage" | sed "s|$aros_tree/||" \
    >"$result/images-before.sha256"

echo "[bench] run the workload on AFS+ and on the Fast File System"
AROS_CTL_STARTUP_MODE=minimal \
AROS_CTL_HOST_FOLDER="$result" \
AROS_CTL_STARTUP_EXTRA="C:FailAt 21
Assign \"FDSK:\" \"SYS:DiskImages\"
C:Mount DEVS:DOSDrivers/AFSPLUS19 >MacRW:mount.out
C:Mount DEVS:DOSDrivers/BASE20 >MacRW:mount-base.out
C:AFSPlusBench FORMAT BASE20: Baseline >MacRW:format.out
C:AFSPlusBench AFSPLUS19:bench $seed $trees >MacRW:afsplus.out
C:AFSPlusBench BASE20:bench $seed $trees >MacRW:baseline.out
C:Mount AFSPLUS19: SHUTDOWN >MacRW:shutdown.out
If WARN
    C:Echo fail >MacRW:shutdown.status
Else
    C:Echo pass >MacRW:shutdown.status
EndIf
C:Echo done >MacRW:done" \
    "$control" run >/dev/null
aros_started=1
# The boot ends with the done file; a run that has not written it by the
# timeout fails rather than being cut short and read as complete.
waited=0
while [ ! -f "$result/done" ]; do
    [ "$waited" -lt "$timeout" ] || {
        echo "[bench] no end of the run after $timeout seconds" >&2
        exit 1
    }
    sleep 5
    waited=$((waited + 5))
done
"$control" status >"$result/status.txt" || true
"$control" stop >/dev/null 2>&1 || true
aros_started=0
cp /tmp/aros-window.log "$result/aros-window.log"
"$repo_root/tools/check-aros-serial-log.sh" "$result/aros-window.log"

for run in afsplus baseline; do
    if grep -q '^\[AFSPLUS-BENCH\] FAIL ' "$result/$run.out"; then
        grep '^\[AFSPLUS-BENCH\] FAIL ' "$result/$run.out" >&2
        exit 1
    fi
    grep -q '^\[AFSPLUS-BENCH\] PASS$' "$result/$run.out"
done
grep -q '^\[AFSPLUS-BENCH\] formatted Baseline$' "$result/format.out"
[ "$(cat "$result/shutdown.status")" = pass ]
# The AFS+ run must carry the handler's counters, the baseline must not.
grep -q '^\[AFSPLUS-BENCH\] counters after ' "$result/afsplus.out"
grep -q '^\[AFSPLUS-BENCH\] counters none$' "$result/baseline.out"

echo "[bench] structural check of the AFS+ image after the run"
cargo run --quiet --release -p afsplus-check --bin afsplus-check -- \
    "$image" --json >"$result/check-after.json"
grep -q '"clean":true' "$result/check-after.json"

cp "$package/SHA256SUMS" "$result/package-SHA256SUMS"
cp "$package/build-profile.txt" "$result/build-profile.txt"
cp "$package/check-before.json" "$result/check-before.json"
python3 tools/bench-bundle.py --result "$result" --seed "$seed" \
    --repo "$repo_root" --mountlist "$package/AFSPLUS19" \
    --baseline-mountlist "$package/BASE20" >"$result/results.json"

mkdir -p "$(dirname -- "$output")"
mv "$result" "$output"
echo "[bench] PASS: $output"
