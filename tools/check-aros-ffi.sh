#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause

set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
macaros_root=${MACAROS_ROOT:-"$repo_root/../Macaros"}
aros_build=${AROS_BUILD:-"$HOME/aros-build"}
aros_crosstools=${AROS_CROSSTOOLS:-"$HOME/aros-crosstools"}
aros_m68k_build=${AROS_M68K_BUILD:-"$HOME/aros-m68k-build"}
rust_toolchain=${AFSPLUS_AROS_RUST_TOOLCHAIN:-nightly-2026-06-27}
handler_output=${AFSPLUS_AROS_HANDLER_OUTPUT:-}
target_json=${AFSPLUS_AROS_RUST_TARGET_JSON:-"$macaros_root/hosted/rust/aarch64-unknown-aros.json"}
target_name=$(basename -- "$target_json" .json)
platform_glue_dir=${AFSPLUS_AROS_PLATFORM_GLUE_DIR:-"$macaros_root/hosted/rust"}
aros_target=${AFSPLUS_AROS_TARGET:-aarch64-unknown-aros}
aros_codegen_target=${AFSPLUS_AROS_CODEGEN_TARGET:-aarch64-unknown-none-elf}
aros_arch_flags=${AFSPLUS_AROS_ARCH_FLAGS:--mcmodel=large -ffixed-x18}
handler_cflags=${AFSPLUS_AROS_HANDLER_CFLAGS:-}
aros_sdk=${AFSPLUS_AROS_SDK_ROOT:-"$aros_build/bin/darwin-aarch64"}
aros_build_tools=${AFSPLUS_AROS_BUILD_TOOLS_ROOT:-"$aros_sdk/tools"}
expected_platform=${AFSPLUS_AROS_EXPECTED_PLATFORM:-}
aros_clang="$aros_crosstools/bin/clang"
aros_ld="$aros_crosstools/bin/ld.lld"
aros_nm="$aros_crosstools/bin/llvm-nm"
aros_objdump=${AFSPLUS_AROS_OBJDUMP:-"$aros_crosstools/bin/llvm-objdump"}
aros_tools="$aros_build_tools"
aros_genmodule="$aros_tools/genmodule"
aros_include="$aros_sdk/AROS/Developer/include"
aros_gen_include="$aros_sdk/gen/include"
aros_stdc_include="$aros_include/aros/stdc"
aros_posixc_include="$aros_gen_include/aros/posixc"
aros_lib="$aros_sdk/AROS/Developer/lib"
aros_cross_lib=${AFSPLUS_AROS_CROSS_LIB:-"$aros_crosstools/lib/generic"}
m68k_cc="$aros_m68k_build/bin/darwin-aarch64/tools/crosstools/m68k-aros-gcc"
m68k_include="$aros_m68k_build/bin/amiga-m68k/AROS/Developer/include"
m68k_gen_include="$aros_m68k_build/bin/amiga-m68k/gen/include"
m68k_stdc_include="$m68k_include/aros/stdc"
archive=${AFSPLUS_AROS_RUST_ARCHIVE:-"${CARGO_TARGET_DIR:-$repo_root/target}/$target_name/release/libafsplus_aros_ffi.a"}
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
require_file "$aros_sdk/gen/config/target.cfg"
require_file "$aros_include/dos/dos64.h"
require_file "$aros_gen_include/aros/config.h"
require_executable "$aros_clang"
require_executable "$aros_ld"
require_executable "$aros_nm"
require_executable "$aros_objdump"
require_executable "$aros_genmodule"
require_executable "$repo_root/tools/check-aros-aarch64-abi.py"
require_file "$aros_lib/libstdc.static.a"
require_file "$aros_cross_lib/libclang_rt.builtins-aarch64.a"
sdk_platform=$(awk '
    $1 == "AROS_TARGET_PLATFORM" && $2 == ":=" { print $3; exit }
' "$aros_sdk/gen/config/target.cfg")
[ -n "$sdk_platform" ] || {
    echo "Missing AROS_TARGET_PLATFORM in SDK target.cfg" >&2
    exit 65
}
if [ -n "$expected_platform" ] && [ "$sdk_platform" != "$expected_platform" ]; then
    echo "AROS SDK platform mismatch: expected $expected_platform, got $sdk_platform" >&2
    exit 65
fi
for glue in \
    aros_net_glue.c aros_fs_glue.c aros_process_glue.c \
    aros_proc_glue.c aros_thread_glue.c aros_sync_glue.c aros_env_glue.c
do
    require_file "$platform_glue_dir/$glue"
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

echo "[aros-ffi] AROS AArch64 profile: sdk=$sdk_platform target=$aros_target codegen=$aros_codegen_target"
echo "[aros-ffi] AROS AArch64 Rust static library"
PATH="$aros_crosstools/bin:$PATH" cargo "+$rust_toolchain" build \
    -p afsplus-aros-ffi --release --target "$target_json" \
    -Zjson-target-spec -Zbuild-std=std,panic_abort

require_file "$archive"
for symbol in \
    afsplus_aros_mount afsplus_aros_unmount afsplus_aros_open \
    afsplus_aros_read afsplus_aros_write afsplus_aros_seek \
    afsplus_aros_set_file_size afsplus_aros_fsync afsplus_aros_rename \
    afsplus_aros_parent_lock_with_access afsplus_aros_lock_from_file \
    afsplus_aros_interface afsplus_aros_capabilities \
    afsplus_aros_set_protection afsplus_aros_read_soft_link \
    afsplus_aros_read_at afsplus_aros_clone_file afsplus_aros_preallocate \
    afsplus_aros_replace afsplus_aros_advise \
    afsplus_aros_watch_add afsplus_aros_watch_drain afsplus_aros_health \
    afsplus_aros_health_events afsplus_aros_set_trace_sink \
    afsplus_aros_trace_counters afsplus_aros_info_json \
    afsplus_aros_counters afsplus_aros_open_from_lock \
    afsplus_aros_change_lock_mode afsplus_aros_change_file_mode \
    afsplus_aros_set_write_protect afsplus_aros_lock_record \
    afsplus_aros_free_record afsplus_aros_lookup_id afsplus_aros_stat_id \
    afsplus_aros_dir_open afsplus_aros_dir_read afsplus_aros_dir_close \
    afsplus_aros_extent_map afsplus_aros_volume_label \
    afsplus_aros_set_volume_label
do
    "$aros_nm" --defined-only "$archive" | grep -Eq "[[:space:]]$symbol$" || {
        echo "Missing exported symbol: $symbol" >&2
        exit 65
    }
done

echo "[aros-ffi] AROS AArch64 C/DOS64 header ABI"
# shellcheck disable=SC2086 -- the profile intentionally supplies separate flags.
"$aros_clang" --target="$aros_target" $aros_arch_flags \
    -std=c11 -Wall -Wextra -Werror \
    -I "$aros_include" -I "$aros_gen_include" -I api \
    -include dos/dos64.h -include afsplus_aros.h \
    -fsyntax-only -x c /dev/null

echo "[aros-ffi] AROS AArch64 DosPacket translator"
for dos64_flag in "" "-D__DOS64=1"; do
    # shellcheck disable=SC2086 -- profile and DOS64 flags are intentional words.
    "$aros_clang" --target="$aros_target" $aros_arch_flags \
        -std=c11 -Wall -Wextra -Werror $dos64_flag \
        -I "$aros_stdc_include" -I "$aros_include" -I "$aros_gen_include" \
        -I api -I native/aros -c native/aros/afsplus_packet.c \
        -o "$task_dir/packet-aarch64${dos64_flag:+-dos64}.o"
done

echo "[aros-ffi] AROS AArch64 bounded trackdisk viewport"
# shellcheck disable=SC2086 -- the profile intentionally supplies separate flags.
"$aros_clang" --target="$aros_target" $aros_arch_flags \
    -std=c11 -Wall -Wextra -Werror \
    -I "$aros_stdc_include" -I "$aros_include" -I "$aros_gen_include" \
    -I api -I native/aros -c native/aros/afsplus_trackdisk.c \
    -o "$task_dir/trackdisk-aarch64.o"

echo "[aros-ffi] AROS AArch64 native handler shell"
# shellcheck disable=SC2086 -- profile and handler flags are separate words.
"$aros_clang" --target="$aros_target" $aros_arch_flags \
    $handler_cflags -std=gnu11 -Wall -Wextra -Werror -D__NOLIBBASE__ \
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
patch -s "$task_dir/module/afsplus_start.c" \
    native/aros/afsplus-handler-autolibs.patch
grep -q 'if (set_open_libraries())' "$task_dir/module/afsplus_start.c"
grep -q 'set_close_libraries();' "$task_dir/module/afsplus_start.c"
for source in afsplus_start afsplus_end; do
    # shellcheck disable=SC2086 -- profile and handler flags are separate words.
    "$aros_clang" --target="$aros_codegen_target" $aros_arch_flags \
        $handler_cflags \
        -D__arm64__ -D__AROS__ \
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
    # shellcheck disable=SC2086 -- the profile intentionally supplies separate flags.
    "$aros_clang" --target="$aros_codegen_target" $aros_arch_flags \
        -D__arm64__ -O2 \
        -Wno-pointer-sign -Wno-int-conversion \
        -Wno-implicit-function-declaration \
        -Wno-incompatible-pointer-types \
        -I "$aros_gen_include" -I "$aros_include" \
        -I "$aros_posixc_include" -I "$aros_stdc_include" \
        -c "$platform_glue_dir/$glue.c" \
        -o "$task_dir/module/$glue.o"
done
for glue in aros_fs_glue aros_sync_glue; do
    # shellcheck disable=SC2086 -- the profile intentionally supplies separate flags.
    "$aros_clang" --target="$aros_codegen_target" $aros_arch_flags \
        -D__arm64__ -O2 \
        -Wno-pointer-sign -Wno-int-conversion \
        -Wno-implicit-function-declaration \
        -Wno-incompatible-library-redeclaration \
        -I "$aros_gen_include" -I "$aros_include" \
        -I "$aros_posixc_include" -I "$aros_stdc_include" \
        -c "$platform_glue_dir/$glue.c" \
        -o "$task_dir/module/$glue.o"
done
PATH="$aros_tools:$PATH" COMPILER_PATH="$aros_crosstools/bin" \
    "$aros_clang" --target="$aros_target" \
    $aros_arch_flags -nostartfiles \
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
"$repo_root/tools/check-aros-aarch64-abi.py" \
    --objdump "$aros_objdump" "$task_dir/afsplus-handler"
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
    # shellcheck disable=SC2086 -- handler flags are intentional words.
    "$m68k_cc" $handler_cflags -O2 -std=gnu11 -Wall -Wextra -Werror \
        -Wno-volatile-register-var -D__NOLIBBASE__ \
        -I "$m68k_stdc_include" -I "$m68k_include" \
        -I "$m68k_gen_include" -I api -I native/aros \
        -c native/aros/afsplus_handler.c \
        -o "$task_dir/handler-m68k.o"
    echo "[aros-ffi] AROS m68k generated handler entry ABI"
    for source in afsplus_start afsplus_end; do
        # shellcheck disable=SC2086 -- handler flags are intentional words.
        "$m68k_cc" $handler_cflags -O2 -std=gnu11 -Wall -Wextra -Werror \
            -Wno-volatile-register-var -D__AROS__ -D__NOLIBBASE__ \
            -Wno-missing-field-initializers -Wno-unused-parameter \
            -Wno-pointer-sign \
            -I "$m68k_include" -I "$m68k_gen_include" \
            -I "$task_dir/module" -c "$task_dir/module/$source.c" \
            -o "$task_dir/module/$source-m68k.o"
    done
fi

if [ -n "$handler_output" ]; then
    handler_output_dir=$(dirname -- "$handler_output")
    mkdir -p "$handler_output_dir"
    [ ! -e "$handler_output" ] || {
        echo "Refusing to replace handler output: $handler_output" >&2
        exit 73
    }
    cp "$task_dir/afsplus-handler" "$handler_output"
    chmod +x "$handler_output"
    echo "[aros-ffi] handler artifact: $handler_output"
fi

echo "[aros-ffi] PASS"
