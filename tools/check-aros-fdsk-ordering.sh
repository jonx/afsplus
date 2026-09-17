#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause

# The one gate that requires a patched AROS.
#
# fdsk.device answers CMD_UPDATE and ETD_UPDATE inside BeginIO, so the reply
# overtakes writes already queued on the unit port: a filesystem's write
# barrier means nothing on the stock device. Every other gate runs against the
# stock device on purpose, because that is what a user has. This one applies
# native/aros/upstream/fdsk-cmd-update.patch to the AROS source tree, rebuilds
# only that device, shows the ordering on a target, runs the Alpha-0 operation
# matrix over the patched device, and puts the tree back as it was.
#
# It refuses to start when the file it patches is already modified, so it can
# never discard somebody else's work, and it restores and rebuilds on every
# exit path.

set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
macaros_root=${MACAROS_ROOT:-"$repo_root/../Macaros"}
aros_source=${AROS_SOURCE:-"$repo_root/../aros-upstream"}
aros_build=${AROS_BUILD:-"$HOME/aros-build"}
aros_crosstools=${AROS_CROSSTOOLS:-"$HOME/aros-crosstools"}
aros_tree="$aros_build/bin/darwin-aarch64/AROS"
control="$macaros_root/graft/aros-ctl"
output=${AFSPLUS_FDSK_ORDERING_OUTPUT:-"$repo_root/build/aros-fdsk-ordering"}
patch_file="$repo_root/native/aros/upstream/fdsk-cmd-update.patch"
patched_source="workbench/devs/fdsk_device.c"
work=$(mktemp -d "${TMPDIR:-/tmp}/afsplus-fdsk-ordering.XXXXXX")
package="$work/package"
result="$work/result"
applied=0
installed=0
aros_started=0

handler="$aros_tree/L/afsplus-handler"
probe="$aros_tree/C/FDSKUpdateProbe"
alpha0="$aros_tree/C/AFSPlusAlpha0Probe"
dosdriver="$aros_tree/Devs/DOSDrivers/AFSPLUS19"
image="$aros_tree/DiskImages/Unit19"
# The probe overwrites block 0 of the unit it is given, so it gets one of its
# own: the AFS+ volume on unit 19 has to survive the run that follows.
scratch="$aros_tree/DiskImages/Unit20"

build_fdsk() {
    PATH="$aros_crosstools/bin:$HOME/aros-build-tools:$PATH" \
        make -C "$aros_build" workbench-devs-fdsk >"$work/build.log" 2>&1
}

cleanup() {
    status=$?
    if [ "$aros_started" = 1 ]; then
        "$control" stop >/dev/null 2>&1 || true
    fi
    if [ "$installed" = 1 ]; then
        for target in "$handler" "$probe" "$alpha0" "$dosdriver" "$image" \
            "$scratch"; do
            [ ! -e "$target" ] || unlink "$target"
        done
    fi
    # The tree goes back to the device a user has, whatever happened.
    if [ "$applied" = 1 ]; then
        if patch -s -R -p1 -d "$aros_source" <"$patch_file"; then
            echo "[fdsk-ordering] patch reverted"
            build_fdsk && echo "[fdsk-ordering] stock fdsk.device rebuilt" \
                || echo "[fdsk-ordering] REBUILD OF THE STOCK DEVICE FAILED:" \
                    " $aros_build is patched-device state" >&2
        else
            echo "[fdsk-ordering] COULD NOT REVERT $patched_source in" \
                "$aros_source; do it by hand before any other gate" >&2
        fi
    fi
    if [ "$status" -ne 0 ] && [ "${AFSPLUS_KEEP_FDSK_FAILURE:-0}" = 1 ]; then
        echo "[fdsk-ordering] keeping failed work directory: $work" >&2
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

require_file "$patch_file"
require_file "$aros_source/$patched_source"
require_file "$repo_root/native/aros/tests/fdsk_update_probe.c"
[ -x "$control" ] || {
    echo "Missing required executable: $control" >&2
    exit 69
}
[ ! -e "$output" ] || {
    echo "Refusing to replace existing result: $output" >&2
    exit 73
}
for target in "$handler" "$probe" "$alpha0" "$dosdriver" "$image" "$scratch"; do
    [ ! -e "$target" ] || {
        echo "Refusing to replace Hosted MacAROS artifact: $target" >&2
        exit 73
    }
done
if ! "$control" status | grep -q '^state=stopped$'; then
    echo "Refusing to disturb a running Hosted MacAROS instance" >&2
    exit 75
fi
# Never discard a local change to the file this gate patches.
if [ -d "$aros_source/.git" ] \
    && ! git -C "$aros_source" diff --quiet -- "$patched_source"; then
    echo "$aros_source/$patched_source is already modified; this gate would" \
        "discard that change" >&2
    exit 73
fi

mkdir "$result"
cd "$repo_root"

echo "[fdsk-ordering] build a fresh qualified package"
AFSPLUS_AROS_PACKAGE_OUTPUT="$package" tools/package-aros-alpha0.sh

echo "[fdsk-ordering] apply $(basename -- "$patch_file") and rebuild fdsk.device"
patch -s -p1 -d "$aros_source" <"$patch_file"
applied=1
build_fdsk

installed=1
cp "$package/afsplus-handler" "$handler"
cp "$package/AFSPlusAlpha0Probe" "$alpha0"
cp "$package/FDSKUpdateProbe" "$probe"
cp "$package/AFSPLUS19" "$dosdriver"
cp "$package/Unit19" "$image"
dd if=/dev/zero of="$scratch" bs=4096 count=16 >/dev/null 2>&1

echo "[fdsk-ordering] the barrier, and AFS+ over the patched device"
AROS_CTL_STARTUP_MODE=minimal \
AROS_CTL_HOST_FOLDER="$result" \
AROS_CTL_STARTUP_EXTRA='C:FailAt 21
Assign "FDSK:" "SYS:DiskImages"
C:FDSKUpdateProbe 20 >MacRW:ordering.out
C:Mount DEVS:DOSDrivers/AFSPLUS19 >MacRW:mount.out
C:AFSPlusAlpha0Probe >MacRW:alpha0.out
C:Mount AFSPLUS19: SHUTDOWN >MacRW:shutdown.out
If WARN
    C:Echo fail >MacRW:shutdown.status
Else
    C:Echo pass >MacRW:shutdown.status
EndIf' \
    "$control" run >/dev/null
aros_started=1
"$control" wait 8 >/dev/null
"$control" status >"$result/status.txt" || true
"$control" stop >/dev/null 2>&1 || true
aros_started=0
cp /tmp/aros-window.log "$result/aros-window.log"
"$repo_root/tools/check-aros-serial-log.sh" "$result/aros-window.log"

grep -q '^\[FDSKUPDATE\] PASS ' "$result/ordering.out"
grep -q '^\[AFSPLUS-ALPHA0\] PASS ' "$result/alpha0.out"
[ "$(cat "$result/shutdown.status")" = pass ]
cargo run --quiet --release -p afsplus-check --bin afsplus-check -- \
    "$image" --json >"$result/check-after.json"
grep -q '"clean":true' "$result/check-after.json"

(
    cd "$result"
    shasum -a 256 ordering.out alpha0.out check-after.json >SHA256SUMS
)
mkdir -p "$(dirname -- "$output")"
mv "$result" "$output"
result=
echo "[fdsk-ordering] PASS: $output"
