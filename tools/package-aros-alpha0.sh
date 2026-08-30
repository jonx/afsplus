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
aros_target=${AFSPLUS_AROS_TARGET:-aarch64-unknown-aros}
aros_codegen_target=${AFSPLUS_AROS_CODEGEN_TARGET:-aarch64-unknown-none-elf}
aros_arch_flags=${AFSPLUS_AROS_ARCH_FLAGS:--mcmodel=large -ffixed-x18}
aros_cross_lib=${AFSPLUS_AROS_CROSS_LIB:-"$aros_crosstools/lib/generic"}
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
[ -x "$sdk/tools/collect-aros" ] || {
    echo "Missing AROS build tree: $sdk" >&2
    exit 69
}
[ -f "$developer/lib/startup.o" ] || {
    echo "Missing AROS program startup: $developer/lib/startup.o" >&2
    exit 69
}

build_target_program() {
    source=$1
    target=$2

    # shellcheck disable=SC2086 -- the platform profile supplies separate flags.
    COMPILER_PATH="$sdk/tools:$aros_crosstools/bin" \
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
        "$developer/lib/startup.o" "$source" \
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
AFSPLUS_AROS_RUST_TARGET_JSON="$target_json" \
AFSPLUS_AROS_PLATFORM_GLUE_DIR="$platform_glue_dir" \
AFSPLUS_AROS_TARGET="$aros_target" \
AFSPLUS_AROS_CODEGEN_TARGET="$aros_codegen_target" \
AFSPLUS_AROS_ARCH_FLAGS="$aros_arch_flags" \
AFSPLUS_AROS_CROSS_LIB="$aros_cross_lib" \
AFSPLUS_AROS_RUST_TOOLCHAIN="$rust_toolchain" \
AFSPLUS_AROS_HANDLER_OUTPUT="$staging/afsplus-handler" \
    tools/check-aros-ffi.sh

echo "[aros-package] build the target-side Alpha-0 probe"
build_target_program native/aros/tests/alpha0_probe.c AFSPlusAlpha0Probe

echo "[aros-package] build the target-side crash-replay probe"
build_target_program native/aros/tests/replay_probe.c AFSPlusReplayProbe

echo "[aros-package] build the target-side S1 system-pivot probe"
build_target_program native/aros/tests/s1_probe.c AFSPlusS1Probe

echo "[aros-package] build the target-side S1b desktop probe"
build_target_program native/aros/tests/s1b_probe.c AFSPlusS1bProbe

echo "[aros-package] build the target-side S1 bootstrap pivot"
build_target_program native/aros/tests/s1_pivot.c AFSPlusS1Pivot

echo "[aros-package] create and verify the 64 MiB image"
cargo run --quiet --release -p afsplus-core --bin afsplus-mkfs -- \
    --size-mib 64 --label AFSPlusAlpha0 "$staging/Unit19"
cargo run --quiet --release -p afsplus-check --bin afsplus-check -- \
    "$staging/Unit19" --json >"$staging/check-before.json"

cp native/aros/AFSPLUS19.mountlist "$staging/AFSPLUS19"
cp docs/aros-alpha0-package.md "$staging/README.md"
{
    echo "format=afsplus-aros-build-profile-v1"
    echo "target=$aros_target"
    echo "codegen_target=$aros_codegen_target"
    echo "arch_flags=$aros_arch_flags"
    echo "rust_toolchain=$rust_toolchain"
    echo "rust_target_json=$(basename -- "$target_json")"
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
    shasum -a 256 afsplus-handler AFSPlusAlpha0Probe AFSPlusReplayProbe \
        AFSPlusS1Probe AFSPlusS1bProbe AFSPlusS1Pivot AFSPLUS19 Unit19 \
        build-profile.txt check-before.json README.md >SHA256SUMS
)

mkdir -p "$(dirname -- "$output")"
mv "$staging" "$output"
staging=

echo "[aros-package] PASS: $output"
echo "[aros-package] no MacAROS tree was modified"
