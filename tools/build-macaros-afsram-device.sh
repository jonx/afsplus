#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause

# Build the external writable retained-image device for native MacAROS QEMU.

set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
aros_crosstools=${AROS_CROSSTOOLS:-"$HOME/aros-crosstools"}
sdk=${AFSPLUS_AROS_SDK_ROOT:-"$HOME/Build/aros-apple-core/apple/bin/apple-aarch64"}
build_tools=${AFSPLUS_AROS_BUILD_TOOLS_ROOT:-"$HOME/Build/aros-apple-core/apple/bin/darwin-aarch64/tools"}
expected_platform=${AFSPLUS_AROS_EXPECTED_PLATFORM:-apple-aarch64}
target=${AFSPLUS_AROS_TARGET:-aarch64-unknown-aros}
codegen_target=${AFSPLUS_AROS_CODEGEN_TARGET:-aarch64-unknown-none-elf}
arch_flags=${AFSPLUS_AROS_ARCH_FLAGS:--mcmodel=large -ffixed-x18}
mount_probe_cflags=${AFSPLUS_AROS_MOUNT_PROBE_CFLAGS:-}
output=${AFSPLUS_AROS_AFSRAM_OUTPUT:-"$repo_root/build/macaros-afsram/afsram.device"}
probe_output=${AFSPLUS_AROS_AFSRAM_PROBE_OUTPUT:-"$(dirname -- "$output")/AFSPlusAfsRamProbe"}
alpha_probe_output=${AFSPLUS_AROS_NATIVE_ALPHA_PROBE_OUTPUT:-"$(dirname -- "$output")/AFSPlusNativeAlpha0Probe"}
mount_probe_output=${AFSPLUS_AROS_NATIVE_MOUNT_PROBE_OUTPUT:-"$(dirname -- "$output")/AFSPlusNativeMountProbe"}
cross_lib=${AFSPLUS_AROS_CROSS_LIB:-"$aros_crosstools/lib/generic"}
clang="$aros_crosstools/bin/clang"
nm="$aros_crosstools/bin/llvm-nm"
objdump=${AFSPLUS_AROS_OBJDUMP:-"$aros_crosstools/bin/llvm-objdump"}
genmodule="$build_tools/genmodule"
developer="$sdk/AROS/Developer"
generated="$sdk/gen/include"
source_dir="$repo_root/native/aros/apple"
task_dir=$(mktemp -d "${TMPDIR:-/tmp}/afsplus-afsram-device.XXXXXX")
trap 'rm -r "$task_dir"' EXIT HUP INT TERM

require_file() {
    [ -f "$1" ] || { echo "Missing required file: $1" >&2; exit 66; }
}

require_executable() {
    [ -x "$1" ] || { echo "Missing required executable: $1" >&2; exit 69; }
}

require_executable "$clang"
require_executable "$nm"
require_executable "$objdump"
require_executable "$genmodule"
require_executable "$repo_root/tools/check-aros-aarch64-abi.py"
require_file "$sdk/gen/config/target.cfg"
require_file "$source_dir/afsram.conf"
require_file "$source_dir/afsram_device.c"
require_file "$source_dir/afsram_format.c"
require_file "$repo_root/native/aros/tests/afsram_probe.c"
require_file "$developer/lib/startup.o"
require_file "$cross_lib/libclang_rt.builtins-aarch64.a"

platform=$(awk '
    $1 == "AROS_TARGET_PLATFORM" && $2 == ":=" { print $3; exit }
' "$sdk/gen/config/target.cfg")
[ "$platform" = "$expected_platform" ] || {
    echo "AROS SDK platform mismatch: expected $expected_platform, got ${platform:-missing}" >&2
    exit 65
}
[ ! -e "$output" ] || {
    echo "Refusing to replace existing device: $output" >&2
    exit 73
}
[ ! -e "${output}.abi-report.txt" ] || {
    echo "Refusing to replace existing ABI report: ${output}.abi-report.txt" >&2
    exit 73
}
[ ! -e "${output}.SHA256SUMS" ] || {
    echo "Refusing to replace existing checksum file: ${output}.SHA256SUMS" >&2
    exit 73
}
[ ! -e "$probe_output" ] || {
    echo "Refusing to replace existing probe: $probe_output" >&2
    exit 73
}
[ ! -e "${probe_output}.abi-report.txt" ] || {
    echo "Refusing to replace existing probe ABI report: ${probe_output}.abi-report.txt" >&2
    exit 73
}
[ ! -e "$alpha_probe_output" ] || {
    echo "Refusing to replace existing native Alpha-0 probe: $alpha_probe_output" >&2
    exit 73
}
[ ! -e "${alpha_probe_output}.abi-report.txt" ] || {
    echo "Refusing to replace existing native Alpha-0 ABI report: ${alpha_probe_output}.abi-report.txt" >&2
    exit 73
}
[ ! -e "$mount_probe_output" ] || {
    echo "Refusing to replace existing native mount probe: $mount_probe_output" >&2
    exit 73
}
[ ! -e "${mount_probe_output}.abi-report.txt" ] || {
    echo "Refusing to replace existing mount-probe ABI report: ${mount_probe_output}.abi-report.txt" >&2
    exit 73
}

echo "[afsram-device] host descriptor parser"
clang -std=c11 -Wall -Wextra -Wconversion -Werror \
    -I "$source_dir" \
    "$source_dir/afsram_format.c" \
    "$repo_root/native/aros/tests/afsram_format_stub.c" \
    -o "$task_dir/afsram-format-stub"
"$task_dir/afsram-format-stub"

mkdir -p "$task_dir/module/include"
"$genmodule" -c "$source_dir/afsram.conf" -d "$task_dir/module" \
    writelibdefs afsram device
"$genmodule" -c "$source_dir/afsram.conf" -d "$task_dir/module" \
    writefiles afsram device

for source in afsram_start afsram_end; do
    # shellcheck disable=SC2086 -- the profile supplies separate ABI flags.
    "$clang" --target="$codegen_target" $arch_flags \
        -D__arm64__ -D__AROS__ -D__NOLIBBASE__ -O2 \
        -Wall -Wextra -Werror -Wno-missing-field-initializers \
        -Wno-unused-parameter -Wno-pointer-sign \
        -I "$generated" -I "$developer/include" -I "$source_dir" \
        -I "$task_dir/module" -c "$task_dir/module/$source.c" \
        -o "$task_dir/module/$source.o"
done

for source in afsram_format afsram_device; do
    # shellcheck disable=SC2086 -- the profile supplies separate ABI flags.
    "$clang" --target="$codegen_target" $arch_flags \
        -D__arm64__ -D__AROS__ -D__NOLIBBASE__ -O2 \
        -DLC_LIBDEFS_FILE='"afsram_libdefs.h"' \
        -std=gnu11 -Wall -Wextra -Wconversion -Wsign-conversion -Werror \
        -Wno-pointer-sign \
        -I "$generated" -I "$developer/include" \
        -I "$generated/aros/posixc" -I "$developer/include/aros/stdc" \
        -I "$source_dir" -I "$task_dir/module/include" \
        -I "$task_dir/module" -c "$source_dir/$source.c" \
        -o "$task_dir/module/$source.o"
done

# shellcheck disable=SC2086 -- the profile supplies separate ABI flags.
PATH="$build_tools:$PATH" COMPILER_PATH="$aros_crosstools/bin" \
    "$clang" --target="$target" $arch_flags -nostartfiles \
    -Wl,--allow-multiple-definition \
    -L "$developer/lib" -o "$task_dir/afsram.device" \
    "$task_dir/module/afsram_start.o" \
    "$task_dir/module/afsram_device.o" \
    "$task_dir/module/afsram_format.o" \
    "$task_dir/module/afsram_end.o" \
    -Wl,--start-group -lamiga -larossupport -laros -lexec \
    -lautoinit -llibinit -Wl,--end-group

if "$nm" --undefined-only "$task_dir/afsram.device" | grep -Eq '[^[:space:]]'; then
    echo "Unresolved symbol in afsram.device:" >&2
    "$nm" --undefined-only "$task_dir/afsram.device" >&2
    exit 65
fi
for symbol in AfsRam_5_AfsRam_BeginIO AfsRam_6_AfsRam_AbortIO AfsRam_ROMTag; do
    "$nm" --defined-only "$task_dir/afsram.device" | \
        grep -Eq "[[:space:]]$symbol$" || {
        echo "Missing afsram.device symbol: $symbol" >&2
        exit 65
    }
done
"$repo_root/tools/check-aros-aarch64-abi.py" \
    --objdump "$objdump" "$task_dir/afsram.device" \
    >"$task_dir/abi-report.txt"

# shellcheck disable=SC2086 -- the profile supplies separate ABI flags.
COMPILER_PATH="$build_tools:$aros_crosstools/bin" \
    "$clang" --target="$target" $arch_flags -O2 -std=gnu11 \
    -DAFSPLUS_AFSRAM_BLOCK_ONLY=1 \
    -Wall -Wextra -Wconversion -Wsign-conversion -Werror \
    -Wno-pointer-sign \
    -isystem "$developer/include" -isystem "$generated" \
    -isystem "$generated/aros/posixc" \
    -isystem "$developer/include/aros/stdc" \
    -I "$source_dir" \
    -nostartfiles -nodefaultlibs \
    -L "$developer/lib" -L "$cross_lib" \
    "$developer/lib/startup.o" \
    "$repo_root/native/aros/tests/afsram_probe.c" \
    -o "$task_dir/AFSPlusAfsRamProbe" \
    -Wl,--allow-multiple-definition -Wl,--start-group \
    -lpthread -lposixc -lstdc -lstdcio -ldos -lexec -laros \
    -lautoinit -llibinit -lutility -lexpansion -lamiga -larossupport \
    -Wl,--end-group -lclang_rt.builtins-aarch64
"$repo_root/tools/check-aros-aarch64-abi.py" \
    --objdump "$objdump" "$task_dir/AFSPlusAfsRamProbe" \
    >"$task_dir/probe-abi-report.txt"

# shellcheck disable=SC2086 -- profile and probe flags are separate words.
COMPILER_PATH="$build_tools:$aros_crosstools/bin" \
    "$clang" --target="$target" $arch_flags -O2 -std=gnu11 \
    $mount_probe_cflags \
    -DAFSPLUS_PROBE_VOLUME='"AFSPLUS0"' \
    -DAFSPLUS_PROBE_FUNCTION=afsplus_native_alpha_probe \
    -Wall -Wextra -Wconversion -Wsign-conversion -Werror \
    -Wno-pointer-sign -Wno-cast-function-type-mismatch \
    -isystem "$developer/include" -isystem "$generated" \
    -isystem "$generated/aros/posixc" \
    -isystem "$developer/include/aros/stdc" \
    -I "$source_dir" \
    -nostartfiles -nodefaultlibs \
    -L "$developer/lib" -L "$cross_lib" \
    "$developer/lib/startup.o" \
    "$repo_root/native/aros/tests/afsram_probe.c" \
    "$repo_root/native/aros/tests/alpha0_probe.c" \
    "$task_dir/module/afsram_format.o" \
    -o "$task_dir/AFSPlusNativeMountProbe" \
    -Wl,--allow-multiple-definition -Wl,--start-group \
    -lpthread -lposixc -lstdc -lstdcio -ldos -lexec -laros \
    -lautoinit -llibinit -lutility -lexpansion -lamiga -larossupport \
    -Wl,--end-group -lclang_rt.builtins-aarch64
"$repo_root/tools/check-aros-aarch64-abi.py" \
    --objdump "$objdump" "$task_dir/AFSPlusNativeMountProbe" \
    >"$task_dir/mount-probe-abi-report.txt"

# shellcheck disable=SC2086 -- the profile supplies separate ABI flags.
COMPILER_PATH="$build_tools:$aros_crosstools/bin" \
    "$clang" --target="$target" $arch_flags -O2 -std=gnu11 \
    -DAFSPLUS_PROBE_VOLUME='"AFSPLUS0"' \
    -Wall -Wextra -Wconversion -Wsign-conversion -Werror \
    -Wno-pointer-sign \
    -isystem "$developer/include" -isystem "$generated" \
    -isystem "$generated/aros/posixc" \
    -isystem "$developer/include/aros/stdc" \
    -nostartfiles -nodefaultlibs \
    -L "$developer/lib" -L "$cross_lib" \
    "$developer/lib/startup.o" \
    "$repo_root/native/aros/tests/alpha0_probe.c" \
    -o "$task_dir/AFSPlusNativeAlpha0Probe" \
    -Wl,--allow-multiple-definition -Wl,--start-group \
    -lpthread -lposixc -lstdc -lstdcio -ldos -lexec -laros \
    -lautoinit -llibinit -lutility -lexpansion -lamiga -larossupport \
    -Wl,--end-group -lclang_rt.builtins-aarch64
"$repo_root/tools/check-aros-aarch64-abi.py" \
    --objdump "$objdump" "$task_dir/AFSPlusNativeAlpha0Probe" \
    >"$task_dir/alpha-probe-abi-report.txt"

mkdir -p "$(dirname -- "$output")"
mv "$task_dir/afsram.device" "$output"
mv "$task_dir/AFSPlusAfsRamProbe" "$probe_output"
mv "$task_dir/AFSPlusNativeAlpha0Probe" "$alpha_probe_output"
mv "$task_dir/AFSPlusNativeMountProbe" "$mount_probe_output"
cp "$task_dir/abi-report.txt" "${output}.abi-report.txt"
cp "$task_dir/probe-abi-report.txt" "${probe_output}.abi-report.txt"
cp "$task_dir/alpha-probe-abi-report.txt" \
    "${alpha_probe_output}.abi-report.txt"
cp "$task_dir/mount-probe-abi-report.txt" \
    "${mount_probe_output}.abi-report.txt"
shasum -a 256 "$output" "$probe_output" "$alpha_probe_output" \
    "$mount_probe_output" \
    "${output}.abi-report.txt" "${probe_output}.abi-report.txt" \
    "${alpha_probe_output}.abi-report.txt" \
    "${mount_probe_output}.abi-report.txt" \
    >"${output}.SHA256SUMS"
cat "${output}.abi-report.txt"
cat "${probe_output}.abi-report.txt"
cat "${alpha_probe_output}.abi-report.txt"
cat "${mount_probe_output}.abi-report.txt"
echo "[afsram-device] PASS: $output"
