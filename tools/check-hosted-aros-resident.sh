#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause

# The AFS+ handler as a boot module on Hosted MacAROS. Its resident init
# registers it in FileSystem.resource for DosType AFS+ (native/aros/afsplus.conf,
# section handler), which is how the boot scan will find a handler for an AFS+
# partition. Proof: a DOSDriver without a FileSystem line mounts an AFS+ volume
# when the handler is in the boot module list, and the same boot without it
# cannot mount the volume.
#
#   AFSPLUS_RESIDENT_OUTPUT   where the result goes (default build/hosted-aros-resident)
#   AFSPLUS_RESIDENT_TIMEOUT  seconds a boot may take (default 120)

set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
macaros_root=${MACAROS_ROOT:-"$repo_root/../Macaros"}
aros_build=${AROS_BUILD:-"$HOME/aros-build"}
aros_tree="$aros_build/bin/darwin-aarch64/AROS"
boot_conf="$aros_tree/boot/darwin/AROSBootstrap.conf"
control="$macaros_root/graft/aros-ctl"
timeout=${AFSPLUS_RESIDENT_TIMEOUT:-120}
output=${AFSPLUS_RESIDENT_OUTPUT:-"$repo_root/build/hosted-aros-resident"}
work=$(mktemp -d "${TMPDIR:-/tmp}/afsplus-resident.XXXXXX")
package="$work/package"
result="$work/result"
aros_started=0
installed=0

handler="$aros_tree/L/afsplus-handler"
info="$aros_tree/C/AFSPlusInfo"
dosdriver="$aros_tree/Devs/DOSDrivers/AFSPLUS19"
image="$aros_tree/DiskImages/Unit19"
module_line="module $handler"

remove_module_line() {
    [ -f "$boot_conf" ] || return 0
    grep -vxF "$module_line" "$boot_conf" >"$work/conf" || true
    cat "$work/conf" >"$boot_conf"
}

cleanup() {
    status=$?
    if [ "$aros_started" = 1 ]; then
        "$control" stop >/dev/null 2>&1 || true
    fi
    remove_module_line
    if [ "$installed" = 1 ]; then
        for target in "$handler" "$info" "$dosdriver" "$image"; do
            [ ! -e "$target" ] || unlink "$target"
        done
    fi
    if [ "$status" -ne 0 ] && [ "${AFSPLUS_KEEP_HOSTED_FAILURE:-0}" = 1 ]; then
        echo "[resident] keeping failed work directory: $work" >&2
        return
    fi
    [ ! -d "$work" ] || rm -r "$work"
}
trap cleanup EXIT HUP INT TERM

[ -x "$control" ] || { echo "Missing $control" >&2; exit 69; }
[ -f "$boot_conf" ] || { echo "Missing $boot_conf" >&2; exit 69; }
[ -f "$aros_tree/Devs/fdsk.device" ] || {
    echo "Missing fdsk.device; run tools/prepare-hosted-aros.sh" >&2
    exit 69
}
[ ! -e "$output" ] || {
    echo "Refusing to replace existing result: $output" >&2
    exit 73
}
for target in "$handler" "$info" "$dosdriver" "$image"; do
    [ ! -e "$target" ] || {
        echo "Refusing to replace Hosted MacAROS artifact: $target" >&2
        exit 73
    }
done
if grep -qxF "$module_line" "$boot_conf"; then
    echo "The boot module list already names $handler" >&2
    exit 73
fi
if ! "$control" status | grep -q '^state=stopped$'; then
    echo "Refusing to disturb a running Hosted MacAROS instance" >&2
    exit 75
fi

mkdir "$result"
cd "$repo_root"

echo "[resident] build a fresh qualified package"
AFSPLUS_AROS_PACKAGE_OUTPUT="$package" tools/package-aros-alpha0.sh
(cd "$package" && shasum -a 256 -c SHA256SUMS >/dev/null)

installed=1
cp "$package/afsplus-handler" "$handler"
cp "$package/AFSPlusInfo" "$info"
cp "$package/Unit19" "$image"
# No FileSystem line: only FileSystem.resource can give the node a handler.
grep -v '^FileSystem' "$package/AFSPLUS19" >"$dosdriver"
if grep -q '^FileSystem' "$dosdriver"; then
    echo "The DOSDriver still names its handler" >&2
    exit 1
fi

# One boot: mount the volume through whatever FileSystem.resource holds, and
# ask it the AFS+ query.
boot() {
    run=$1
    mkdir "$result/$run"
    AROS_CTL_STARTUP_MODE=minimal \
    AROS_CTL_HOST_FOLDER="$result/$run" \
    AROS_CTL_STARTUP_EXTRA="C:FailAt 21
Assign \"FDSK:\" \"SYS:DiskImages\"
C:Mount DEVS:DOSDrivers/AFSPLUS19 >MacRW:mount.out
If WARN
    C:Echo mount refused >MacRW:info.out
Else
    C:AFSPlusInfo AFSPLUS19: >MacRW:info.out
EndIf
C:Echo done >MacRW:done" \
        "$control" run >/dev/null
    aros_started=1
    waited=0
    while [ ! -f "$result/$run/done" ]; do
        [ "$waited" -lt "$timeout" ] || {
            echo "[resident] $run: no end of the boot after $timeout seconds" >&2
            exit 1
        }
        sleep 2
        waited=$((waited + 2))
    done
    "$control" stop >/dev/null 2>&1 || true
    aros_started=0
    cp /tmp/aros-window.log "$result/$run/aros-window.log"
}

echo "[resident] boot with the handler in the boot module list"
printf '%s\n' "$module_line" >>"$boot_conf"
boot resident
remove_module_line
grep -q '^{"schema":"afsplus-handler-info"' "$result/resident/info.out" || {
    echo "[resident] the registered handler did not serve the volume:" >&2
    cat "$result/resident/mount.out" "$result/resident/info.out" >&2
    exit 1
}
grep -q '"label":"AFSPlusAlpha0"' "$result/resident/info.out"

echo "[resident] the same boot without it"
boot control
if [ "$(cat "$result/control/info.out")" != "mount refused" ]; then
    echo "[resident] the volume mounted without the boot module" >&2
    exit 1
fi

echo "[resident] structural check of the AFS+ image"
cargo run --quiet --release -p afsplus-check --bin afsplus-check -- \
    "$image" --json >"$result/check-after.json"
grep -q '"clean":true' "$result/check-after.json"

mkdir -p "$(dirname -- "$output")"
mv "$result" "$output"
echo "[resident] PASS: the boot module registers AFS+ in FileSystem.resource"
