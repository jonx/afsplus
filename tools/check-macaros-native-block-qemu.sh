#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause

# Qualify the external writable retained-image transport under QEMU. Alpha-0
# mode also mounts the off-tree handler and runs its complete operation matrix.

set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
native_repo=${MACAROS_NATIVE_ROOT:-"$repo_root/../aros-apple-silicon"}
core_build=${MACAROS_NATIVE_BUILD:-"$HOME/Build/aros-apple-core/apple"}
core_source=${AROS_CORE_SOURCE:-"$repo_root/../aros-apple-core"}
sdk=${AFSPLUS_AROS_SDK_ROOT:-"$core_build/bin/apple-aarch64"}
build_tools=${AFSPLUS_AROS_BUILD_TOOLS_ROOT:-"$core_build/bin/darwin-aarch64/tools"}
output=${AFSPLUS_MACAROS_QEMU_OUTPUT:-"$repo_root/build/macaros-native-block-qemu"}
mode=${AFSPLUS_MACAROS_QEMU_MODE:-block}
handler_cflags=${AFSPLUS_AROS_HANDLER_CFLAGS:-}
efi="$native_repo/boot/arosboot/build/AROSBOOTAA64.EFI"
stage2="$core_build/bin/apple-aarch64/gen/arch/aarch64-apple/bootstrap/aros-apple-stage2-test.bin"

require_file() {
    [ -f "$1" ] || { echo "Missing required file: $1" >&2; exit 66; }
}

require_executable() {
    [ -x "$1" ] || { echo "Missing required executable: $1" >&2; exit 69; }
}

[ ! -e "$output" ] || {
    echo "Refusing to replace existing QEMU evidence: $output" >&2
    exit 73
}
case "$mode" in
block|image|alpha0) ;;
*) echo "AFSPLUS_MACAROS_QEMU_MODE must be block, image or alpha0" >&2; exit 64 ;;
esac
if [ "$mode" = alpha0 ] && [ -z "$handler_cflags" ]; then
    handler_cflags=-DAFSPLUS_AROS_TRACE_STARTUP=1
fi
require_executable "$native_repo/tools/arosbundle"
require_executable "$native_repo/tools/make-fat12-image.py"
require_executable "$native_repo/boot/arosboot/test-qemu.sh"
require_file "$efi"
require_file "$stage2"

native_status_before=$(git -C "$native_repo" status --porcelain=v1)
core_status_before=$(git -C "$core_source" status --porcelain=v1)
mkdir -p "$output"

AFSPLUS_AROS_PACKAGE_OUTPUT="$output/alpha0" \
AFSPLUS_AROS_SDK_ROOT="$sdk" \
AFSPLUS_AROS_BUILD_TOOLS_ROOT="$build_tools" \
AFSPLUS_AROS_EXPECTED_PLATFORM=apple-aarch64 \
AFSPLUS_AROS_PROFILE_ID=macaros-native-apple-aarch64-prehardware \
AFSPLUS_AROS_HANDLER_CFLAGS="$handler_cflags" \
    "$repo_root/tools/package-aros-alpha0.sh"

AFSPLUS_AROS_SDK_ROOT="$sdk" \
AFSPLUS_AROS_BUILD_TOOLS_ROOT="$build_tools" \
AFSPLUS_AROS_EXPECTED_PLATFORM=apple-aarch64 \
AFSPLUS_AROS_MOUNT_PROBE_CFLAGS="${AFSPLUS_AROS_MOUNT_PROBE_CFLAGS:-}" \
AFSPLUS_AROS_AFSRAM_OUTPUT="$output/transport/afsram.device" \
    "$repo_root/tools/build-macaros-afsram-device.sh"

system_probe="$output/transport/AFSPlusAfsRamProbe"
if [ "$mode" = alpha0 ] || [ "$mode" = image ]; then
    if [ "$mode" = alpha0 ]; then
        system_probe="$output/transport/AFSPlusNativeMountProbe"
    fi
    "$repo_root/tools/make-macaros-afsplus-system.py" \
        --abi-probe "$system_probe" \
        --device "$output/transport/afsram.device" \
        "$output/sys-probe-device.fixture"
else
    python3 -B "$native_repo/tools/make-fat12-image.py" \
        --abi-probe "$output/transport/AFSPlusAfsRamProbe" \
        "$output/sys-probe.fixture"
    "$repo_root/tools/inject-fat12-file.py" \
        --file "$output/transport/afsram.device" \
        --path DEVS/afsram.device \
        "$output/sys-probe.fixture" "$output/sys-probe-device.fixture"
fi
"$repo_root/tools/make-macaros-afsram-image.py" \
    --system-image "$output/sys-probe-device.fixture" \
    --aros-handler "$output/alpha0/afsplus-handler" \
    --afsplus-image "$output/alpha0/Unit19" \
    "$output/sys-composite.img"

set -- "$native_repo/tools/arosbundle" create \
    --output "$output/AFSRAM-QEMU.BND" \
    --kernel "$stage2" --kernel-name aros-apple-stage2 \
    --kernel-memory-size 16777216 \
    --component "kernel.resource=$core_build/bin/apple-aarch64/AROS/boot/apple/Devs/kernel.resource" \
    --component "exec.library=$core_build/bin/apple-aarch64/AROS/boot/apple/Libs/exec.library" \
    --component "expansion.library=$core_build/bin/apple-aarch64/AROS/boot/apple/Libs/expansion.library" \
    --component "pmgr.resource=$core_build/bin/apple-aarch64/AROS/Devs/pmgr.resource" \
    --component "utility.library=$core_build/bin/apple-aarch64/AROS/Libs/utility.library" \
    --component "rtkit.resource=$core_build/bin/apple-aarch64/AROS/Devs/rtkit.resource" \
    --component "aros.library=$core_build/bin/apple-aarch64/AROS/Libs/aros.library" \
    --component "bootloader.resource=$core_build/bin/apple-aarch64/AROS/Devs/bootloader.resource" \
    --component "task.resource=$core_build/bin/apple-aarch64/AROS/Devs/task.resource" \
    --component "FileSystem.resource=$core_build/bin/apple-aarch64/AROS/Devs/FileSystem.resource" \
    --component "timer.device=$core_build/bin/apple-aarch64/AROS/boot/apple/Devs/timer.device" \
    --component "partition.library=$core_build/bin/apple-aarch64/AROS/Libs/partition.library" \
    --component "locale.library=$core_build/bin/apple-aarch64/AROS/Libs/locale.library" \
    --component "stdc.library=$core_build/bin/apple-aarch64/AROS/Libs/stdc.library" \
    --component "stdcio.library=$core_build/bin/apple-aarch64/AROS/Libs/stdcio.library" \
    --component "posixc.library=$core_build/bin/apple-aarch64/AROS/Libs/posixc.library" \
    --component "fat-handler=$core_build/bin/apple-aarch64/AROS/L/fat-handler" \
    --component "appleboot.device=$core_build/bin/apple-aarch64/AROS/Devs/appleboot.device" \
    --component "dos.library=$core_build/bin/apple-aarch64/AROS/Libs/dos.library" \
    --component "iffparse.library=$core_build/bin/apple-aarch64/AROS/Libs/iffparse.library" \
    --ramdisk "$output/sys-composite.img"
"$@"
"$native_repo/tools/arosbundle" verify "$output/AFSRAM-QEMU.BND"

RUNDIR="$output/run" \
EXPECTED_MODULES=22 \
EXPECTED_ENTRY_MARKER='[B2Q] AROS startup contract PASS' \
EXPECTED_ENTRY_FAILURE_MARKER='[B2Q] FAIL:' \
BUNDLE_IMAGE="$output/AFSRAM-QEMU.BND" \
EFI_IMAGE="$efi" \
    "$native_repo/boot/arosboot/test-qemu.sh"

grep -q '^\[B2A\] portable ABI + emulated TLS + streams + W^X/unload PASS$' \
    "$output/run/semihost.log" || {
    echo "Native AFSRAM probe did not satisfy the ABI/unload gate" >&2
    exit 1
}
native_status_after=$(git -C "$native_repo" status --porcelain=v1)
core_status_after=$(git -C "$core_source" status --porcelain=v1)
[ "$native_status_before" = "$native_status_after" ] || {
    echo "Native integration worktree changed during the QEMU gate" >&2
    exit 1
}
[ "$core_status_before" = "$core_status_after" ] || {
    echo "AROS core worktree changed during the QEMU gate" >&2
    exit 1
}

{
    echo "format=afsplus-macaros-native-qemu-v2"
    echo "result=PASS"
    echo "mode=$mode"
    echo "hardware_claim=none"
    echo "transport=retained-ram-image"
    echo "descriptor_version=2"
    if [ "$mode" = alpha0 ]; then
        echo "filesystem=afsplus-handler"
        echo "operations=create,read,write,sparse-write,truncate,rename,fsync,casefold,case-only-rename,dismount,unload"
    elif [ "$mode" = image ]; then
        echo "filesystem=present-not-mounted"
        echo "operations=open,read,write-identical,update,reread,geometry"
    else
        echo "filesystem=none"
        echo "operations=open,read,write-identical,update,reread,geometry"
    fi
    printf 'bundle_sha256='
    shasum -a 256 "$output/AFSRAM-QEMU.BND" | awk '{print $1}'
    printf 'composite_sha256='
    shasum -a 256 "$output/sys-composite.img" | awk '{print $1}'
    printf 'device_sha256='
    shasum -a 256 "$output/transport/afsram.device" | awk '{print $1}'
    printf 'probe_sha256='
    shasum -a 256 "$system_probe" | awk '{print $1}'
    printf 'handler_sha256='
    shasum -a 256 "$output/alpha0/afsplus-handler" | awk '{print $1}'
    printf 'kernel_resource_sha256='
    shasum -a 256 "$core_build/bin/apple-aarch64/AROS/boot/apple/Devs/kernel.resource" | awk '{print $1}'
    printf 'stage2_sha256='
    shasum -a 256 "$stage2" | awk '{print $1}'
} >"$output/report.txt"

echo "macaros-native-$mode-qemu result=PASS evidence=$output"
