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
aros_ld="$aros_crosstools/bin/ld.lld"
aros_nm="$aros_crosstools/bin/llvm-nm"
aros_tools="$aros_build/bin/darwin-aarch64/tools"
aros_genmodule="$aros_tools/genmodule"
aros_include="$aros_build/bin/darwin-aarch64/AROS/Developer/include"
aros_gen_include="$aros_build/bin/darwin-aarch64/gen/include"
aros_stdc_include="$aros_include/aros/stdc"
aros_posixc_include="$aros_gen_include/aros/posixc"
aros_lib="$aros_build/bin/darwin-aarch64/AROS/Developer/lib"
aros_cross_lib="$aros_crosstools/lib/generic"
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
require_executable "$aros_ld"
require_executable "$aros_nm"
require_executable "$aros_genmodule"
require_file "$aros_lib/libstdc.static.a"
require_file "$aros_cross_lib/libclang_rt.builtins-aarch64.a"
for glue in \
    aros_net_glue.c aros_fs_glue.c aros_process_glue.c \
    aros_proc_glue.c aros_thread_glue.c aros_sync_glue.c aros_env_glue.c
do
    require_file "$macaros_root/hosted/rust/$glue"
done

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

echo "[aros-ffi] host bounded trackdisk viewport matrix"
clang -std=c11 -Wall -Wextra -Werror \
    -D__WORDSIZE=64 -DAROS_FAST_BPTR=1 -DAROS_FAST_BSTR=1 \
    -I "$aros_stdc_include" -I "$aros_include" -I "$aros_gen_include" \
    -I api -I native/aros \
    native/aros/afsplus_trackdisk.c native/aros/tests/trackdisk_stub.c \
    -o "$task_dir/trackdisk-stub"
"$task_dir/trackdisk-stub"

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

echo "[aros-ffi] AROS AArch64 bounded trackdisk viewport"
"$aros_clang" --target=aarch64-unknown-aros -mcmodel=large -ffixed-x18 \
    -std=c11 -Wall -Wextra -Werror \
    -I "$aros_stdc_include" -I "$aros_include" -I "$aros_gen_include" \
    -I api -I native/aros -c native/aros/afsplus_trackdisk.c \
    -o "$task_dir/trackdisk-aarch64.o"

echo "[aros-ffi] AROS AArch64 native handler shell"
"$aros_clang" --target=aarch64-unknown-aros -mcmodel=large -ffixed-x18 \
    -std=gnu11 -Wall -Wextra -Werror -D__NOLIBBASE__ \
    -I "$aros_stdc_include" -I "$aros_include" -I "$aros_gen_include" \
    -I api -I native/aros -c native/aros/afsplus_handler.c \
    -o "$task_dir/handler-aarch64.o"

echo "[aros-ffi] AROS AArch64 relocatable handler/staticlib link"
"$aros_ld" -r "$task_dir/handler-aarch64.o" \
    "$task_dir/packet-aarch64.o" "$task_dir/trackdisk-aarch64.o" \
    "$archive" -o "$task_dir/handler-bundle-aarch64.o"
for symbol in handler afsplus_aros_mount afsplus_aros_packet_process; do
    "$aros_nm" --defined-only "$task_dir/handler-bundle-aarch64.o" \
        | grep -Eq "[[:space:]]$symbol$" || {
        echo "Missing linked handler symbol: $symbol" >&2
        exit 65
    }
done
if "$aros_nm" --undefined-only "$task_dir/handler-bundle-aarch64.o" \
    | grep -Eq '[[:space:]]afsplus(_aros)?_[[:alnum:]_]+$'; then
    echo "Unresolved AFS+ symbol in linked handler bundle:" >&2
    "$aros_nm" --undefined-only "$task_dir/handler-bundle-aarch64.o" \
        | grep -E '[[:space:]]afsplus(_aros)?_[[:alnum:]_]+$' >&2
    exit 65
fi

echo "[aros-ffi] AROS AArch64 complete generated handler module"
mkdir -p "$task_dir/module/include"
"$aros_genmodule" -c native/aros/afsplus.conf -d "$task_dir/module" \
    writelibdefs afsplus handler
"$aros_genmodule" -c native/aros/afsplus.conf -d "$task_dir/module" \
    writefiles afsplus handler
for source in afsplus_start afsplus_end; do
    "$aros_clang" --target=aarch64-unknown-none-elf \
        -mcmodel=large -ffixed-x18 -D__arm64__ -D__AROS__ \
        -D__NOLIBBASE__ -O2 -Wall -Wextra -Werror \
        -Wno-missing-field-initializers -Wno-unused-parameter \
        -Wno-pointer-sign \
        -I "$aros_gen_include" -I "$aros_include" \
        -I "$task_dir/module" -c "$task_dir/module/$source.c" \
        -o "$task_dir/module/$source.o"
done
for glue in \
    aros_net_glue aros_process_glue aros_proc_glue \
    aros_env_glue aros_thread_glue
do
    "$aros_clang" --target=aarch64-unknown-none-elf \
        -mcmodel=large -ffixed-x18 -D__arm64__ -O2 \
        -Wno-pointer-sign -Wno-int-conversion \
        -Wno-implicit-function-declaration \
        -Wno-incompatible-pointer-types \
        -I "$aros_gen_include" -I "$aros_include" \
        -c "$macaros_root/hosted/rust/$glue.c" \
        -o "$task_dir/module/$glue.o"
done
for glue in aros_fs_glue aros_sync_glue; do
    "$aros_clang" --target=aarch64-unknown-none-elf \
        -mcmodel=large -ffixed-x18 -D__arm64__ -O2 \
        -Wno-pointer-sign -Wno-int-conversion \
        -Wno-implicit-function-declaration \
        -Wno-incompatible-library-redeclaration \
        -I "$aros_gen_include" -I "$aros_include" \
        -I "$aros_posixc_include" \
        -c "$macaros_root/hosted/rust/$glue.c" \
        -o "$task_dir/module/$glue.o"
done
PATH="$aros_tools:$PATH" COMPILER_PATH="$aros_crosstools/bin" \
    "$aros_clang" --target=aarch64-unknown-aros \
    -mcmodel=large -ffixed-x18 -nostartfiles \
    -Wl,--allow-multiple-definition \
    -L "$aros_lib" -L "$aros_cross_lib" \
    -o "$task_dir/afsplus-handler" \
    "$task_dir/module/afsplus_start.o" \
    "$task_dir/handler-aarch64.o" \
    "$task_dir/packet-aarch64.o" \
    "$task_dir/trackdisk-aarch64.o" \
    "$task_dir/module"/aros_*_glue.o \
    "$archive" "$task_dir/module/afsplus_end.o" \
    -Wl,--start-group \
    -lstdc.static -lmui -lamiga -larossupport -lamiga -lcodesets \
    -lkeymap -lexpansion -lcommodities -ldiskfont -lasl -lmuimaster \
    -ldatatypes -lcybergraphics -lworkbench -licon -lintuition \
    -lgadtools -llayers -laros -lpartition -liffparse -lgraphics \
    -llocale -ldos -lutility -loop -llibinit -lautoinit \
    -lposixc -lstdcio -lstdc -lexec -lpthread \
    -lclang_rt.builtins-aarch64 -Wl,--end-group
if "$aros_nm" --undefined-only "$task_dir/afsplus-handler" \
    | grep -Eq '[^[:space:]]'; then
    echo "Unresolved symbol in complete AROS handler module:" >&2
    "$aros_nm" --undefined-only "$task_dir/afsplus-handler" >&2
    exit 65
fi
for symbol in afsplus_Handler handler afsplus_aros_mount; do
    "$aros_nm" --defined-only "$task_dir/afsplus-handler" \
        | grep -Eq "[[:space:]]$symbol$" || {
        echo "Missing complete handler symbol: $symbol" >&2
        exit 65
    }
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
    echo "[aros-ffi] AROS m68k bounded trackdisk viewport"
    "$m68k_cc" -std=c11 -Wall -Wextra -Werror \
        -I "$m68k_stdc_include" -I "$m68k_include" \
        -I "$m68k_gen_include" -I api -I native/aros \
        -c native/aros/afsplus_trackdisk.c \
        -o "$task_dir/trackdisk-m68k.o"
    echo "[aros-ffi] AROS m68k native handler shell"
    # The SDK's register-call inlines trigger this GCC warning at every call
    # site; suppress only that header/toolchain diagnostic and retain -Werror.
    "$m68k_cc" -O2 -std=gnu11 -Wall -Wextra -Werror \
        -Wno-volatile-register-var -D__NOLIBBASE__ \
        -I "$m68k_stdc_include" -I "$m68k_include" \
        -I "$m68k_gen_include" -I api -I native/aros \
        -c native/aros/afsplus_handler.c \
        -o "$task_dir/handler-m68k.o"
    echo "[aros-ffi] AROS m68k generated handler entry ABI"
    for source in afsplus_start afsplus_end; do
        "$m68k_cc" -O2 -std=gnu11 -Wall -Wextra -Werror \
            -Wno-volatile-register-var -D__AROS__ -D__NOLIBBASE__ \
            -Wno-missing-field-initializers -Wno-unused-parameter \
            -Wno-pointer-sign \
            -I "$m68k_include" -I "$m68k_gen_include" \
            -I "$task_dir/module" -c "$task_dir/module/$source.c" \
            -o "$task_dir/module/$source-m68k.o"
    done
fi

echo "[aros-ffi] PASS"
