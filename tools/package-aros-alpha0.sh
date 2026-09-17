#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause

# Build a self-contained, host-checked MacAROS Alpha-0 qualification package.
# This script never installs into or starts a MacAROS tree.

set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
output=${AFSPLUS_AROS_PACKAGE_OUTPUT:-"$repo_root/build/aros-alpha0"}
aros_build=${AROS_BUILD:-"$HOME/aros-build"}
aros_crosstools=${AROS_CROSSTOOLS:-"$HOME/aros-crosstools"}
sdk=${AFSPLUS_AROS_SDK_ROOT:-"$aros_build/bin/darwin-aarch64"}
build_tools=${AFSPLUS_AROS_BUILD_TOOLS_ROOT:-"$sdk/tools"}
expected_platform=${AFSPLUS_AROS_EXPECTED_PLATFORM:-}
profile_id=${AFSPLUS_AROS_PROFILE_ID:-}
aros_target=${AFSPLUS_AROS_TARGET:-aarch64-unknown-aros}
aros_codegen_target=${AFSPLUS_AROS_CODEGEN_TARGET:-aarch64-unknown-none-elf}
aros_arch_flags=${AFSPLUS_AROS_ARCH_FLAGS:--mcmodel=large -ffixed-x18}
handler_cflags=${AFSPLUS_AROS_HANDLER_CFLAGS:-}
aros_cross_lib=${AFSPLUS_AROS_CROSS_LIB:-"$aros_crosstools/lib/generic"}
aros_objdump=${AFSPLUS_AROS_OBJDUMP:-"$aros_crosstools/bin/llvm-objdump"}
rust_toolchain=${AFSPLUS_AROS_RUST_TOOLCHAIN:-nightly-2026-06-27}
macaros_root=${MACAROS_ROOT:-"$repo_root/../Macaros"}
target_json=${AFSPLUS_AROS_RUST_TARGET_JSON:-"$macaros_root/hosted/rust/aarch64-unknown-aros.json"}
platform_glue_dir=${AFSPLUS_AROS_PLATFORM_GLUE_DIR:-"$macaros_root/hosted/rust"}
aros_clang="$aros_crosstools/bin/clang"
developer="$sdk/AROS/Developer"
staging=$(mktemp -d "${TMPDIR:-/tmp}/afsplus-aros-package.XXXXXX")

cleanup() {
    if [ -n "$staging" ] && [ -d "$staging" ]; then
        rm -r "$staging"
    fi
}
trap cleanup EXIT HUP INT TERM

[ ! -e "$output" ] || {
    echo "Refusing to replace existing package: $output" >&2
    exit 73
}
[ -x "$aros_clang" ] || {
    echo "Missing AROS compiler: $aros_clang" >&2
    exit 69
}
[ -x "$build_tools/collect-aros" ] || {
    echo "Missing AROS collect-aros tool: $build_tools/collect-aros" >&2
    exit 69
}
[ -x "$build_tools/genmodule" ] || {
    echo "Missing AROS genmodule tool: $build_tools/genmodule" >&2
    exit 69
}
[ -f "$sdk/gen/config/target.cfg" ] || {
    echo "Missing AROS SDK target config: $sdk/gen/config/target.cfg" >&2
    exit 69
}
[ -f "$developer/lib/startup.o" ] || {
    echo "Missing AROS program startup: $developer/lib/startup.o" >&2
    exit 69
}
[ -x "$aros_objdump" ] || {
    echo "Missing AROS objdump: $aros_objdump" >&2
    exit 69
}
[ -x "$repo_root/tools/check-aros-aarch64-abi.py" ] || {
    echo "Missing AROS AArch64 ABI audit" >&2
    exit 69
}
sdk_platform=$(awk '
    $1 == "AROS_TARGET_PLATFORM" && $2 == ":=" { print $3; exit }
' "$sdk/gen/config/target.cfg")
[ -n "$sdk_platform" ] || {
    echo "Missing AROS_TARGET_PLATFORM in SDK target.cfg" >&2
    exit 65
}
if [ -n "$expected_platform" ] && [ "$sdk_platform" != "$expected_platform" ]; then
    echo "AROS SDK platform mismatch: expected $expected_platform, got $sdk_platform" >&2
    exit 65
fi
profile_id=${profile_id:-$sdk_platform}

build_target_program() {
    source=$1
    target=$2
    # Further sources are linked into the same program.
    shift 2

    # shellcheck disable=SC2086 -- the platform profile supplies separate flags.
    COMPILER_PATH="$build_tools:$aros_crosstools/bin" \
        "$aros_clang" --target="$aros_target" $aros_arch_flags \
        -O2 -std=gnu11 \
        -Wall -Wextra -Wconversion -Wsign-conversion -Werror \
        -Wno-pointer-sign \
        -isystem "$developer/include" \
        -isystem "$sdk/gen/include" \
        -isystem "$sdk/gen/include/aros/posixc" \
        -isystem "$developer/include/aros/stdc" \
        -nostartfiles -nodefaultlibs \
        -L "$developer/lib" -L "$aros_cross_lib" \
        -I api \
        "$developer/lib/startup.o" "$source" "$@" \
        -o "$staging/$target" \
        -Wl,--allow-multiple-definition -Wl,--start-group \
        -lpthread -lposixc -lstdc -lstdcio -ldos -lexec -laros \
        -lautoinit -llibinit -lutility -lamiga -larossupport \
        -Wl,--end-group -lclang_rt.builtins-aarch64
    chmod 755 "$staging/$target"
}

cd "$repo_root"

echo "[aros-package] qualify and materialize the complete handler"
AFSPLUS_AROS_SDK_ROOT="$sdk" \
AFSPLUS_AROS_BUILD_TOOLS_ROOT="$build_tools" \
AFSPLUS_AROS_EXPECTED_PLATFORM="$sdk_platform" \
AFSPLUS_AROS_OBJDUMP="$aros_objdump" \
AFSPLUS_AROS_RUST_TARGET_JSON="$target_json" \
AFSPLUS_AROS_PLATFORM_GLUE_DIR="$platform_glue_dir" \
AFSPLUS_AROS_TARGET="$aros_target" \
AFSPLUS_AROS_CODEGEN_TARGET="$aros_codegen_target" \
AFSPLUS_AROS_ARCH_FLAGS="$aros_arch_flags" \
AFSPLUS_AROS_HANDLER_CFLAGS="$handler_cflags" \
AFSPLUS_AROS_CROSS_LIB="$aros_cross_lib" \
AFSPLUS_AROS_RUST_TOOLCHAIN="$rust_toolchain" \
AFSPLUS_AROS_HANDLER_OUTPUT="$staging/afsplus-handler" \
    tools/check-aros-ffi.sh
"$repo_root/tools/check-aros-aarch64-abi.py" \
    --objdump "$aros_objdump" "$staging/afsplus-handler" \
    >"$staging/abi-report.txt"

echo "[aros-package] build the target-side Alpha-0 probe"
build_target_program native/aros/tests/alpha0_probe.c AFSPlusAlpha0Probe

echo "[aros-package] build the target-side DOS semantics probe"
build_target_program native/aros/tests/dos_compat_probe.c AFSPlusDosProbe \
    native/aros/client/afsplus_client.c

echo "[aros-package] build the target-side handler report tool"
build_target_program native/aros/tools/afsplus_info.c AFSPlusInfo \
    native/aros/client/afsplus_client.c

echo "[aros-package] build the target-side clone-or-copy tool"
build_target_program native/aros/tools/afsplus_clone.c AFSPlusClone \
    native/aros/client/afsplus_client.c native/aros/client/afsplus_copy.c

echo "[aros-package] build the target-side crash-replay probe"
build_target_program native/aros/tests/replay_probe.c AFSPlusReplayProbe

echo "[aros-package] build the target-side S1 system-pivot probe"
build_target_program native/aros/tests/s1_probe.c AFSPlusS1Probe

echo "[aros-package] build the target-side S1b desktop probe"
build_target_program native/aros/tests/s1b_probe.c AFSPlusS1bProbe

echo "[aros-package] build the target-side S1 bootstrap pivot"
build_target_program native/aros/tests/s1_pivot.c AFSPlusS1Pivot

echo "[aros-package] create and verify the 64 MiB image"
cargo run --quiet --release -p afsplus-tools --bin mkafsplus -- \
    --profile workstation --size-mib 64 --label AFSPlusAlpha0 \
    --case-insensitive "$staging/Unit19"
cargo run --quiet --release -p afsplus-check --bin afsplus-check -- \
    "$staging/Unit19" --json >"$staging/check-before.json"

cp native/aros/AFSPLUS19.mountlist "$staging/AFSPLUS19"
cp docs/aros-alpha0-package.md "$staging/README.md"
{
    echo "format=afsplus-aros-build-profile-v2"
    echo "profile_id=$profile_id"
    echo "sdk_platform=$sdk_platform"
    echo "target=$aros_target"
    echo "codegen_target=$aros_codegen_target"
    echo "arch_flags=$aros_arch_flags"
    echo "handler_cflags=$handler_cflags"
    echo "rust_toolchain=$rust_toolchain"
    echo "rust_target_json=$(basename -- "$target_json")"
    printf 'sdk_target_config_sha256='
    shasum -a 256 "$sdk/gen/config/target.cfg" | awk '{print $1}'
    printf 'collect_aros_sha256='
    shasum -a 256 "$build_tools/collect-aros" | awk '{print $1}'
    printf 'genmodule_sha256='
    shasum -a 256 "$build_tools/genmodule" | awk '{print $1}'
    printf 'abi_audit_sha256='
    shasum -a 256 "$repo_root/tools/check-aros-aarch64-abi.py" | awk '{print $1}'
    printf 'rust_target_json_sha256='
    shasum -a 256 "$target_json" | awk '{print $1}'
    echo "platform_glue_sha256_begin"
    (
        cd "$platform_glue_dir"
        shasum -a 256 \
            aros_net_glue.c aros_fs_glue.c aros_process_glue.c \
            aros_proc_glue.c aros_thread_glue.c aros_sync_glue.c \
            aros_env_glue.c
    )
    echo "platform_glue_sha256_end"
} >"$staging/build-profile.txt"
(
    cd "$staging"
    shasum -a 256 afsplus-handler AFSPlusAlpha0Probe AFSPlusDosProbe \
        AFSPlusInfo AFSPlusClone \
        AFSPlusReplayProbe \
        AFSPlusS1Probe AFSPlusS1bProbe AFSPlusS1Pivot AFSPLUS19 Unit19 \
        abi-report.txt build-profile.txt check-before.json README.md \
        >SHA256SUMS
)

mkdir -p "$(dirname -- "$output")"
mv "$staging" "$output"
staging=

echo "[aros-package] PASS: $output"
echo "[aros-package] no MacAROS tree was modified"
