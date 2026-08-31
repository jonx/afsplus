#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause

# Qualify the external writable retained-image transport under QEMU. Alpha-0
# and replay modes mount the off-tree handler, then optionally extract and
# check the exact mutated payload from file-backed guest RAM.

set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
native_repo=${MACAROS_NATIVE_ROOT:-"$repo_root/../aros-apple-silicon"}
core_build=${MACAROS_NATIVE_BUILD:-"$HOME/Build/aros-apple-core/apple"}
sdk=${AFSPLUS_AROS_SDK_ROOT:-"$core_build/bin/apple-aarch64"}
build_tools=${AFSPLUS_AROS_BUILD_TOOLS_ROOT:-"$core_build/bin/darwin-aarch64/tools"}
output=${AFSPLUS_MACAROS_QEMU_OUTPUT:-"$repo_root/build/macaros-native-block-qemu"}
mode=${AFSPLUS_MACAROS_QEMU_MODE:-block}
handler_cflags=${AFSPLUS_AROS_HANDLER_CFLAGS:-}
payload_image=${AFSPLUS_MACAROS_QEMU_AFSPLUS_IMAGE:-}
extract_after=${AFSPLUS_MACAROS_QEMU_EXTRACT:-0}
qemu_real=${AFSPLUS_QEMU_REAL_BIN:-${QEMU:-qemu-system-aarch64}}
efi="$native_repo/boot/arosboot/build/AROSBOOTAA64.EFI"
stage2="$core_build/bin/apple-aarch64/gen/arch/aarch64-apple/bootstrap/aros-apple-stage2-test.bin"

require_file() {
    [ -f "$1" ] || { echo "Missing required file: $1" >&2; exit 66; }
}

require_executable() {
    [ -x "$1" ] || { echo "Missing required executable: $1" >&2; exit 69; }
}

critical_input_fingerprint() {
    shasum -a 256 \
        "$native_repo/tools/arosbundle" \
        "$native_repo/tools/make-fat12-image.py" \
        "$native_repo/boot/arosboot/test-qemu.sh" \
        "$native_repo/tools/boot_generation.py" \
        "$native_repo/harness/qmp.py" \
        "$efi" "$stage2" \
        "$build_tools/genmodule" \
        "$sdk/gen/config/target.cfg"
}

[ ! -e "$output" ] || {
    echo "Refusing to replace existing QEMU evidence: $output" >&2
    exit 73
}
case "$mode" in
block|image|alpha0|replay-old|replay-new) ;;
*) echo "AFSPLUS_MACAROS_QEMU_MODE must be block, image, alpha0, replay-old or replay-new" >&2; exit 64 ;;
esac
case "$extract_after" in
0|1) ;;
*) echo "AFSPLUS_MACAROS_QEMU_EXTRACT must be 0 or 1" >&2; exit 64 ;;
esac
case "$mode" in
replay-old|replay-new)
    [ -n "$payload_image" ] || {
        echo "Replay mode requires AFSPLUS_MACAROS_QEMU_AFSPLUS_IMAGE" >&2
        exit 64
    }
    extract_after=1
    ;;
esac
if [ -z "$payload_image" ]; then
    payload_image="$output/alpha0/Unit19"
fi
require_executable "$native_repo/tools/arosbundle"
require_executable "$native_repo/tools/make-fat12-image.py"
require_executable "$native_repo/boot/arosboot/test-qemu.sh"
require_file "$native_repo/tools/boot_generation.py"
require_file "$native_repo/harness/qmp.py"
require_executable "$repo_root/tools/check-aros-serial-log.sh"
require_executable "$repo_root/tools/qemu-file-backed-memory.sh"
require_executable "$repo_root/tools/extract-macaros-afsram.py"
require_executable "$build_tools/genmodule"
require_file "$efi"
require_file "$stage2"
require_file "$sdk/gen/config/target.cfg"

mkdir -p "$output"
critical_inputs_before=$(critical_input_fingerprint)
printf '%s\n' "$critical_inputs_before" >"$output/critical-inputs.txt"

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
case "$mode" in
image|alpha0|replay-old|replay-new)
    case "$mode" in
    alpha0)
        system_probe="$output/transport/AFSPlusNativeMountProbe"
        ;;
    replay-old)
        system_probe="$output/transport/AFSPlusNativeReplayOldProbe"
        ;;
    replay-new)
        system_probe="$output/transport/AFSPlusNativeReplayNewProbe"
        ;;
    esac
    "$repo_root/tools/make-macaros-afsplus-system.py" \
        --abi-probe "$system_probe" \
        --device "$output/transport/afsram.device" \
        "$output/sys-probe-device.fixture"
    ;;
block)
    python3 -B "$native_repo/tools/make-fat12-image.py" \
        --abi-probe "$output/transport/AFSPlusAfsRamProbe" \
        "$output/sys-probe.fixture"
    "$repo_root/tools/inject-fat12-file.py" \
        --file "$output/transport/afsram.device" \
        --path DEVS/afsram.device \
        "$output/sys-probe.fixture" "$output/sys-probe-device.fixture"
    ;;
esac
require_file "$payload_image"
"$repo_root/tools/make-macaros-afsram-image.py" \
    --system-image "$output/sys-probe-device.fixture" \
    --aros-handler "$output/alpha0/afsplus-handler" \
    --afsplus-image "$payload_image" \
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

if [ "$extract_after" = 1 ]; then
    QEMU="$repo_root/tools/qemu-file-backed-memory.sh" \
    AFSPLUS_QEMU_REAL_BIN="$qemu_real" \
    AFSPLUS_QEMU_MEMORY_FILE="$output/guest-memory.bin" \
    QEMU_MACHINE='virt,acpi=off,memory-backend=afsplus-memory' \
    RUNDIR="$output/run" \
    EXPECTED_MODULES=22 \
    EXPECTED_ENTRY_MARKER='[B2Q] AROS startup contract PASS' \
    EXPECTED_ENTRY_FAILURE_MARKER='[B2Q] FAIL:' \
    BUNDLE_IMAGE="$output/AFSRAM-QEMU.BND" \
    EFI_IMAGE="$efi" \
        "$native_repo/boot/arosboot/test-qemu.sh"
else
    RUNDIR="$output/run" \
    EXPECTED_MODULES=22 \
    EXPECTED_ENTRY_MARKER='[B2Q] AROS startup contract PASS' \
    EXPECTED_ENTRY_FAILURE_MARKER='[B2Q] FAIL:' \
    BUNDLE_IMAGE="$output/AFSRAM-QEMU.BND" \
    EFI_IMAGE="$efi" \
        "$native_repo/boot/arosboot/test-qemu.sh"
fi

"$repo_root/tools/check-aros-serial-log.sh" "$output/run/serial.log"
"$repo_root/tools/check-aros-serial-log.sh" "$output/run/semihost.log"

grep -q '^\[B2A\] portable ABI + emulated TLS + streams + W^X/unload PASS$' \
    "$output/run/semihost.log" || {
    echo "Native AFSRAM probe did not satisfy the ABI/unload gate" >&2
    exit 1
}
checker_clean=not-run
if [ "$extract_after" = 1 ]; then
    "$repo_root/tools/extract-macaros-afsram.py" \
        "$output/guest-memory.bin" "$output/payload-after.img"
    unlink "$output/guest-memory.bin"
    cargo run --quiet --release -p afsplus-check --bin afsplus-check -- \
        "$output/payload-after.img" --json >"$output/check-after.json"
    grep -q '"clean":true' "$output/check-after.json"
    grep -q '"log_records_pending":0' "$output/check-after.json"
    checker_clean=true
fi
critical_inputs_after=$(critical_input_fingerprint)
[ "$critical_inputs_before" = "$critical_inputs_after" ] || {
    echo "A consumed native-QEMU input changed during the gate" >&2
    exit 1
}
critical_inputs_sha256=$(printf '%s\n' "$critical_inputs_before" | \
    shasum -a 256 | awk '{print $1}')

{
    echo "format=afsplus-macaros-native-qemu-v2"
    echo "result=PASS"
    echo "mode=$mode"
    echo "hardware_claim=none"
    echo "transport=retained-ram-image"
    echo "descriptor_version=2"
    echo "checker_clean=$checker_clean"
    echo "guest_failure_requester=none"
    echo "critical_inputs_sha256=$critical_inputs_sha256"
    if [ "$mode" = alpha0 ]; then
        echo "filesystem=afsplus-handler"
        echo "operations=create,read,write,sparse-write,truncate,rename,fsync,casefold,case-only-rename,dismount,unload"
    elif [ "$mode" = replay-old ] || [ "$mode" = replay-new ]; then
        echo "filesystem=afsplus-handler"
        echo "replay_expected=${mode#replay-}"
        echo "operations=mount-replay,read-expected-state,dismount,unload,extract,strict-check"
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
    if [ "$extract_after" = 1 ]; then
        printf 'payload_after_sha256='
        shasum -a 256 "$output/payload-after.img" | awk '{print $1}'
        printf 'check_after_sha256='
        shasum -a 256 "$output/check-after.json" | awk '{print $1}'
    fi
    printf 'kernel_resource_sha256='
    shasum -a 256 "$core_build/bin/apple-aarch64/AROS/boot/apple/Devs/kernel.resource" | awk '{print $1}'
    printf 'stage2_sha256='
    shasum -a 256 "$stage2" | awk '{print $1}'
} >"$output/report.txt"

echo "macaros-native-$mode-qemu result=PASS evidence=$output"
