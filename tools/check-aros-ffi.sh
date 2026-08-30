#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause

set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
macaros_root=${MACAROS_ROOT:-"$repo_root/../Macaros"}
aros_build=${AROS_BUILD:-"$HOME/aros-build"}
aros_crosstools=${AROS_CROSSTOOLS:-"$HOME/aros-crosstools"}
aros_m68k_build=${AROS_M68K_BUILD:-"$HOME/aros-m68k-build"}
rust_toolchain=${AFSPLUS_AROS_RUST_TOOLCHAIN:-nightly-2026-06-27}
target_json="$macaros_root/hosted/rust/aarch64-unknown-aros.json"
aros_clang="$aros_crosstools/bin/clang"
aros_nm="$aros_crosstools/bin/llvm-nm"
aros_include="$aros_build/bin/darwin-aarch64/AROS/Developer/include"
aros_gen_include="$aros_build/bin/darwin-aarch64/gen/include"
aros_stdc_include="$aros_include/aros/stdc"
m68k_cc="$aros_m68k_build/bin/darwin-aarch64/tools/crosstools/m68k-aros-gcc"
m68k_include="$aros_m68k_build/bin/amiga-m68k/AROS/Developer/include"
m68k_gen_include="$aros_m68k_build/bin/amiga-m68k/gen/include"
m68k_stdc_include="$m68k_include/aros/stdc"
archive="$repo_root/target/aarch64-unknown-aros/release/libafsplus_aros_ffi.a"
task_dir=$(mktemp -d "${TMPDIR:-/tmp}/afsplus-aros-ffi.XXXXXX")
trap 'rm -r "$task_dir"' EXIT HUP INT TERM

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

require_file "$target_json"
require_file "$aros_include/dos/dos64.h"
require_file "$aros_gen_include/aros/config.h"
require_executable "$aros_clang"
require_executable "$aros_nm"

cd "$repo_root"

echo "[aros-ffi] host Rust operation matrix"
cargo test -p afsplus-aros-ffi --all-features

echo "[aros-ffi] host C11 header"
clang -std=c11 -Wall -Wextra -Werror -I api \
    -include afsplus_aros.h -fsyntax-only -x c /dev/null

echo "[aros-ffi] host DosPacket translation matrix"
clang -std=c11 -Wall -Wextra -Werror \
    -D__WORDSIZE=64 -DAROS_FAST_BPTR=1 -DAROS_FAST_BSTR=1 \
    -I "$aros_stdc_include" -I "$aros_include" -I "$aros_gen_include" \
    -I api -I native/aros \
    native/aros/afsplus_packet.c native/aros/tests/packet_stub.c \
    -o "$task_dir/packet-stub"
"$task_dir/packet-stub"

echo "[aros-ffi] AROS AArch64 Rust static library"
PATH="$aros_crosstools/bin:$PATH" cargo "+$rust_toolchain" build \
    -p afsplus-aros-ffi --release --target "$target_json" \
    -Zjson-target-spec -Zbuild-std=std,panic_abort

require_file "$archive"
for symbol in \
    afsplus_aros_mount afsplus_aros_unmount afsplus_aros_open \
    afsplus_aros_read afsplus_aros_write afsplus_aros_seek \
    afsplus_aros_set_file_size afsplus_aros_fsync afsplus_aros_rename \
    afsplus_aros_parent_lock_with_access afsplus_aros_lock_from_file
do
    "$aros_nm" --defined-only "$archive" | grep -Eq "[[:space:]]$symbol$" || {
        echo "Missing exported symbol: $symbol" >&2
        exit 65
    }
done

echo "[aros-ffi] AROS AArch64 C/DOS64 header ABI"
"$aros_clang" --target=aarch64-unknown-aros -mcmodel=large -ffixed-x18 \
    -std=c11 -Wall -Wextra -Werror \
    -I "$aros_include" -I "$aros_gen_include" -I api \
    -include dos/dos64.h -include afsplus_aros.h \
    -fsyntax-only -x c /dev/null

echo "[aros-ffi] AROS AArch64 DosPacket translator"
for dos64_flag in "" "-D__DOS64=1"; do
    # shellcheck disable=SC2086 -- the empty/non-empty compile flag is intentional.
    "$aros_clang" --target=aarch64-unknown-aros -mcmodel=large -ffixed-x18 \
        -std=c11 -Wall -Wextra -Werror $dos64_flag \
        -I "$aros_stdc_include" -I "$aros_include" -I "$aros_gen_include" \
        -I api -I native/aros -c native/aros/afsplus_packet.c \
        -o "$task_dir/packet-aarch64${dos64_flag:+-dos64}.o"
done

if [ "${AFSPLUS_AROS_SKIP_M68K_ABI:-0}" != 1 ]; then
    require_executable "$m68k_cc"
    require_file "$m68k_include/dos/dos64.h"
    require_file "$m68k_gen_include/aros/config.h"
    echo "[aros-ffi] AROS m68k C/DOS64 header ABI"
    "$m68k_cc" -std=c11 -Wall -Wextra -Werror \
        -I "$m68k_include" -I "$m68k_gen_include" -I api \
        -include dos/dos64.h -include afsplus_aros.h \
        -fsyntax-only -x c /dev/null
    echo "[aros-ffi] AROS m68k DosPacket translator"
    "$m68k_cc" -std=c11 -Wall -Wextra -Werror \
        -I "$m68k_stdc_include" -I "$m68k_include" \
        -I "$m68k_gen_include" -I api -I native/aros \
        -c native/aros/afsplus_packet.c -o "$task_dir/packet-m68k.o"
fi

echo "[aros-ffi] PASS"
