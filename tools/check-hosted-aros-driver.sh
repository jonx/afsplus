#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause

# The block-device contract of docs/aros-block-device-contract.md on Hosted
# MacAROS: AFSPlusDriverProbe runs against fdsk.device and hostdisk.device,
# each over a scratch image. Two runs must fail: one with
# CORRUPT, which flips a byte of the expected pattern, at read-back; one over
# a range past the end of the device, at range. The images must be the same
# afterwards, byte for byte: the probe writes the range back. The stock
# devices fail one clause, the barrier, as a known defect; it is recorded,
# and every other clause must hold.
#
#   AFSPLUS_DRIVER_OUTPUT   where the result goes (default build/hosted-aros-driver)
#   AFSPLUS_DRIVER_TIMEOUT  seconds the boot may take (default 120)

set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
macaros_root=${MACAROS_ROOT:-"$repo_root/../Macaros"}
aros_build=${AROS_BUILD:-"$HOME/aros-build"}
aros_tree="$aros_build/bin/darwin-aarch64/AROS"
control="$macaros_root/graft/aros-ctl"
timeout=${AFSPLUS_DRIVER_TIMEOUT:-120}
output=${AFSPLUS_DRIVER_OUTPUT:-"$repo_root/build/hosted-aros-driver"}
work=$(mktemp -d "${TMPDIR:-/tmp}/afsplus-driver.XXXXXX")
package="$work/package"
result="$work/result"
aros_started=0
installed=0

probe="$aros_tree/C/AFSPlusDriverProbe"
fdsk_image="$aros_tree/DiskImages/Unit21"
host_image="$work/disk0.img"

cleanup() {
    status=$?
    if [ "$aros_started" = 1 ]; then
        "$control" stop >/dev/null 2>&1 || true
    fi
    if [ "$installed" = 1 ]; then
        for target in "$probe" "$fdsk_image"; do
            [ ! -e "$target" ] || unlink "$target"
        done
    fi
    if [ "$status" -ne 0 ] && [ "${AFSPLUS_KEEP_HOSTED_FAILURE:-0}" = 1 ]; then
        echo "[driver] keeping failed work directory: $work" >&2
        return
    fi
    [ ! -d "$work" ] || rm -r "$work"
}
trap cleanup EXIT HUP INT TERM

[ -x "$control" ] || { echo "Missing $control" >&2; exit 69; }
for path in Devs/fdsk.device Devs/hostdisk.device; do
    [ -f "$aros_tree/$path" ] || { echo "Missing $aros_tree/$path" >&2; exit 69; }
done
[ ! -e "$output" ] || {
    echo "Refusing to replace existing result: $output" >&2
    exit 73
}
for target in "$probe" "$fdsk_image"; do
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

echo "[driver] build a fresh qualified package"
AFSPLUS_AROS_PACKAGE_OUTPUT="$package" tools/package-aros-alpha0.sh
(cd "$package" && shasum -a 256 -c SHA256SUMS >/dev/null)

installed=1
cp "$package/AFSPlusDriverProbe" "$probe"
# 64 MiB of a repeating non-zero pattern, so a restore to zeros shows.
python3 - "$fdsk_image" "$host_image" <<'EOF'
import sys
pattern = bytes(range(256)) * 16
for path in sys.argv[1:]:
    with open(path, "wb") as image:
        for _ in range(64 * 1024 * 1024 // len(pattern)):
            image.write(pattern)
EOF
shasum -a 256 "$fdsk_image" | cut -d' ' -f1 >"$result/fdsk-before.sha256"
shasum -a 256 "$host_image" | cut -d' ' -f1 >"$result/host-before.sha256"

echo "[driver] probe fdsk.device and hostdisk.device"
AROS_HOST_ARGS="hostdisk=$work/disk%ld.img" \
AROS_CTL_STARTUP_MODE=minimal \
AROS_CTL_HOST_FOLDER="$result" \
AROS_CTL_STARTUP_EXTRA='C:FailAt 21
Assign "FDSK:" "SYS:DiskImages"
C:AFSPlusDriverProbe fdsk.device 21 16 8 WRITE >MacRW:fdsk.out
C:AFSPlusDriverProbe hostdisk.device 0 16 8 WRITE >MacRW:hostdisk.out
C:AFSPlusDriverProbe fdsk.device 21 16 8 WRITE CORRUPT >MacRW:corrupt.out
C:AFSPlusDriverProbe fdsk.device 21 16380 8 WRITE >MacRW:range.out
C:Echo done >MacRW:done' \
    "$control" run >/dev/null
aros_started=1
waited=0
while [ ! -f "$result/done" ]; do
    [ "$waited" -lt "$timeout" ] || {
        echo "[driver] no end of the boot after $timeout seconds" >&2
        exit 1
    }
    sleep 2
    waited=$((waited + 2))
done
"$control" stop >/dev/null 2>&1 || true
aros_started=0
cp /tmp/aros-window.log "$result/aros-window.log"

# Both stock devices answer CMD_UPDATE inside BeginIO, ahead of a queued
# write: a known defect of fdsk (stage-c-gap C12, fixed by
# native/aros/upstream/fdsk-cmd-update.patch and proven by
# check-aros-fdsk-ordering.sh) and the same in hostdisk. Every other clause
# must hold; the barrier result is recorded, and a device that starts to
# pass it is reported.
: >"$result/known-defects.txt"
for device in fdsk hostdisk; do
    others=$(grep '^\[AFSPLUS-DRIVER\] FAIL ' "$result/$device.out" \
        | grep -v '^\[AFSPLUS-DRIVER\] FAIL barrier ' || true)
    [ -z "$others" ] && grep -q '^\[AFSPLUS-DRIVER\] PASS read-back$' "$result/$device.out" || {
        echo "[driver] $device.device does not meet the contract:" >&2
        cat "$result/$device.out" >&2
        exit 1
    }
    if grep -q '^\[AFSPLUS-DRIVER\] FAIL barrier ' "$result/$device.out"; then
        echo "$device.device barrier" >>"$result/known-defects.txt"
    else
        echo "[driver] $device.device now keeps CMD_UPDATE behind queued writes"
    fi
done
grep -q '^\[AFSPLUS-DRIVER\] FAIL read-back ' "$result/corrupt.out"
tail -n 1 "$result/corrupt.out" | grep -qx '\[AFSPLUS-DRIVER\] FAIL'
grep -q '^\[AFSPLUS-DRIVER\] FAIL range ' "$result/range.out"
tail -n 1 "$result/range.out" | grep -qx '\[AFSPLUS-DRIVER\] FAIL'

[ "$(shasum -a 256 "$fdsk_image" | cut -d' ' -f1)" = "$(cat "$result/fdsk-before.sha256")" ] || {
    echo "[driver] the probe left the fdsk image changed" >&2
    exit 1
}
[ "$(shasum -a 256 "$host_image" | cut -d' ' -f1)" = "$(cat "$result/host-before.sha256")" ] || {
    echo "[driver] the probe left the hostdisk image changed" >&2
    exit 1
}

mkdir -p "$(dirname -- "$output")"
mv "$result" "$output"
echo "[driver] PASS: fdsk.device and hostdisk.device meet the block-device contract but for: $(tr '\n' ',' <"$output/known-defects.txt")"
