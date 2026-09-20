#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause

# Build the AFS+ handler and its four programs for one platform profile and
# lay them out as a drawer Pkg can publish: L/, C/, Devs/DOSDrivers/, a ReadMe
# a person can act on, the ABI audit, the build profile and SHA256SUMS.
#
#   tools/package-aros-dist.sh <profile> <output-dir>
#
# Profiles live in native/aros/profiles/. This script never installs into and
# never starts an AROS tree, and it signs nothing: signing and publishing are
# the channel maintainer's, with a key this script must not see.

set -eu

usage() {
    echo "usage: tools/package-aros-dist.sh <profile> <output-dir>" >&2
    echo "profiles:" >&2
    for candidate in "$repo_root"/native/aros/profiles/*.sh; do
        echo "  $(basename -- "$candidate" .sh)" >&2
    done
    exit 64
}

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
[ $# -eq 2 ] || usage
profile=$1
output=$2

if [ "$profile" = m68k ]; then
    # Said plainly rather than left to fail somewhere in the middle: the m68k
    # handler is a complete build, but only with the Rust toolchain whose LLVM
    # carries the m68k backend patch (native/aros/toolchain/). On a machine
    # that has it, tools/check-aros-m68k-alpha0-fsuae.sh builds and runs it.
    # This script would otherwise produce a drawer with no handler in it.
    cat >&2 <<'EOF'
package-aros-dist: there is no m68k profile here yet.
The m68k handler needs the patched Rust toolchain (m68k-ccr-fixed) that
tools/check-aros-m68k-alpha0-fsuae.sh uses; tools/check-aros-ffi.sh only
compiles and links the C shell for m68k, which is not a handler.
EOF
    exit 69
fi

profile_file="$repo_root/native/aros/profiles/$profile.sh"
[ -f "$profile_file" ] || {
    echo "package-aros-dist: no such profile: $profile" >&2
    usage
}
[ ! -e "$output" ] || {
    echo "package-aros-dist: refusing to replace $output" >&2
    exit 73
}

# shellcheck disable=SC1090 -- the profile is named on the command line.
. "$profile_file"

rust_toolchain=$aros_rust_toolchain
target_name=$(basename -- "$aros_rust_target_json" .json)
archive=${AFSPLUS_AROS_RUST_ARCHIVE:-"${CARGO_TARGET_DIR:-$repo_root/target}/$target_name/release/libafsplus_aros_ffi.a"}

for needed in "$aros_cc" "$aros_collect_aros" "$aros_genmodule" "$aros_nm" \
    "$aros_objcopy" "$aros_abi_audit"
do
    [ -x "$needed" ] || {
        echo "package-aros-dist: missing executable: $needed" >&2
        exit 69
    }
done
for needed in "$aros_rust_target_json" "$aros_startup"; do
    [ -f "$needed" ] || {
        echo "package-aros-dist: missing file: $needed" >&2
        exit 69
    }
done
# The thread and sync glues are deliberately absent: the handler answers what
# std asks of them itself, in native/aros/afsplus_bootthread.c, so that it
# links no thread library and carries no thread table.
for glue in aros_net_glue aros_fs_glue aros_process_glue aros_proc_glue \
    aros_env_glue
do
    [ -f "$aros_platform_glue_dir/$glue.c" ] || {
        echo "package-aros-dist: missing std glue: $glue.c" >&2
        exit 69
    }
done

staging=$(mktemp -d "${TMPDIR:-/tmp}/afsplus-aros-dist.XXXXXX")
work=$(mktemp -d "${TMPDIR:-/tmp}/afsplus-aros-dist-work.XXXXXX")
cleanup() {
    [ -z "$staging" ] || rm -rf "$staging"
    [ -z "$work" ] || rm -rf "$work"
}
trap cleanup EXIT HUP INT TERM

cd "$repo_root"
mkdir -p "$staging/L" "$staging/C" "$staging/Devs/DOSDrivers" \
    "$work/obj" "$work/module/include"

echo "[aros-dist] $profile_id: AFS+ static library for $target_name"
rust_flags=${RUSTFLAGS:-}
[ -z "$aros_rust_cpu_features" ] || \
    rust_flags="$rust_flags -C target-feature=$aros_rust_cpu_features"
RUSTFLAGS="$rust_flags" cargo "+$rust_toolchain" build \
    -p afsplus-aros-ffi --release --target "$aros_rust_target_json" \
    -Zjson-target-spec -Zbuild-std=std,panic_abort
[ -f "$archive" ] || {
    echo "package-aros-dist: the Rust build left no $archive" >&2
    exit 65
}

echo "[aros-dist] $profile_id: handler shell"
# shellcheck disable=SC2086 -- every profile value is a list of flags.
for source in afsplus_handler afsplus_bootlibc afsplus_bootthread \
    afsplus_packet afsplus_trackdisk afsplus_claim afsplus_control
do
    "$aros_cc" --target="$aros_target" $aros_arch_flags $aros_defines \
        -O2 -std=gnu11 -Wall -Wextra -Werror -D__NOLIBBASE__ \
        $aros_includes -I api -I native/aros \
        -c "native/aros/$source.c" -o "$work/obj/$source.o"
done
# The boot POSIX shims deliberately redefine what the headers declare.
# shellcheck disable=SC2086 -- every profile value is a list of flags.
"$aros_cc" --target="$aros_target" $aros_arch_flags $aros_defines \
    -O2 -std=gnu11 -Wall -Wextra -Werror -D__NOLIBBASE__ -fno-builtin \
    $aros_includes -I api -I native/aros \
    -c native/aros/afsplus_bootposix.c -o "$work/obj/afsplus_bootposix.o"

echo "[aros-dist] $profile_id: generated module entry"
"$aros_genmodule" -c native/aros/afsplus.conf -d "$work/module" \
    writelibdefs afsplus handler
"$aros_genmodule" -c native/aros/afsplus.conf -d "$work/module" \
    writefiles afsplus handler
patch -s "$work/module/afsplus_start.c" \
    native/aros/afsplus-handler-autolibs.patch
# The patch is what makes the handler open its libraries and refuse a startup
# it cannot serve; a silently unapplied patch would be a handler that crashes
# on the target instead of failing here.
grep -q 'if (set_open_libraries() && set_call_funcs' \
    "$work/module/afsplus_start.c"
grep -q 'set_close_libraries();' "$work/module/afsplus_start.c"
grep -q 'afsplus_aros_refuse_startup(SysBase);' "$work/module/afsplus_start.c"
for source in afsplus_start afsplus_end; do
    # shellcheck disable=SC2086 -- every profile value is a list of flags.
    "$aros_cc" --target="$aros_codegen_target" $aros_arch_flags \
        $aros_codegen_defines -D__NOLIBBASE__ -O2 -Wall -Wextra -Werror \
        -Wno-missing-field-initializers -Wno-unused-parameter \
        -Wno-pointer-sign \
        $aros_includes -I "$work/module" \
        -c "$work/module/$source.c" -o "$work/module/$source.o"
done

echo "[aros-dist] $profile_id: AROS Rust std glue"
for glue in aros_net_glue aros_process_glue aros_proc_glue aros_env_glue \
    aros_fs_glue
do
    # These are the MacAROS std glues, compiled as they were written; the
    # warnings they raise are the AROS headers' own.
    # shellcheck disable=SC2086 -- every profile value is a list of flags.
    "$aros_cc" --target="$aros_codegen_target" $aros_arch_flags \
        $aros_codegen_defines -O2 \
        -Wno-pointer-sign -Wno-int-conversion \
        -Wno-implicit-function-declaration -Wno-incompatible-pointer-types \
        -Wno-incompatible-library-redeclaration \
        $aros_glue_includes \
        -c "$aros_platform_glue_dir/$glue.c" -o "$work/module/$glue.o"
done

echo "[aros-dist] $profile_id: link the handler module"
# shellcheck disable=SC2086 -- every profile value is a list of flags.
COMPILER_PATH="$(dirname -- "$aros_cc")" \
    "$aros_collect_aros" -o "$staging/L/afsplus-handler" \
    "$work/module/afsplus_start.o" \
    "$work/obj/afsplus_handler.o" \
    "$work/obj/afsplus_bootlibc.o" "$work/obj/afsplus_bootposix.o" \
    "$work/obj/afsplus_bootthread.o" \
    "$work/obj/afsplus_packet.o" "$work/obj/afsplus_trackdisk.o" \
    "$work/obj/afsplus_claim.o" "$work/obj/afsplus_control.o" \
    "$work/module"/aros_*_glue.o \
    "$archive" "$work/module/afsplus_end.o" \
    $aros_lib_dirs --allow-multiple-definition $aros_module_libs
chmod 755 "$staging/L/afsplus-handler"

for symbol in afsplus_Handler handler afsplus_aros_mount \
    afsplus_aros_packet_process
do
    "$aros_nm" --defined-only "$staging/L/afsplus-handler" \
        | grep -Eq "[[:space:]]$symbol$" || {
        echo "package-aros-dist: missing handler symbol: $symbol" >&2
        exit 65
    }
done
# The handler serves packets without stdc.library: nothing in it may reach an
# allocator or an I/O stub that only exists once a program has a shell.
if "$aros_nm" --defined-only "$staging/L/afsplus-handler" \
    | grep -Eq '__(malloc|calloc|realloc|free|arc4random_buf)_StdCBase_wrapper$'
then
    echo "package-aros-dist: the allocator still goes through stdc.library" >&2
    exit 65
fi
if "$aros_nm" --defined-only "$staging/L/afsplus-handler" \
    | grep -Eq '[[:space:]](PosixCBase|StdCIOBase)$'
then
    echo "package-aros-dist: the handler still links posixc or stdcio" >&2
    exit 65
fi

echo "[aros-dist] $profile_id: the four programs"
program_dir=$staging/C
build_program() {
    name=$1
    shift
    objects=""
    for source in "$@"; do
        base=$(basename -- "$source" .c)
        if [ ! -f "$work/obj/prog-$base.o" ]; then
            # shellcheck disable=SC2086 -- profile values are lists of flags.
            "$aros_cc" --target="$aros_target" $aros_arch_flags \
                $aros_defines -O2 -std=gnu11 \
                -Wall -Wextra -Wconversion -Wsign-conversion -Werror \
                -Wno-pointer-sign \
                $aros_program_includes -I api -I native/aros \
                -c "$source" -o "$work/obj/prog-$base.o"
        fi
        objects="$objects $work/obj/prog-$base.o"
    done
    # shellcheck disable=SC2086 -- every profile value is a list of flags.
    COMPILER_PATH="$(dirname -- "$aros_cc")" \
        "$aros_collect_aros" -o "$program_dir/$name" "$aros_startup" $objects \
        $aros_lib_dirs --allow-multiple-definition $aros_program_libs
    chmod 755 "$program_dir/$name"
}

for entry in \
    "AFSPlusInfo native/aros/tools/afsplus_info.c native/aros/client/afsplus_client.c" \
    "AFSPlusTour native/aros/tools/afsplus_tour.c native/aros/client/afsplus_client.c" \
    "AFSPlusClone native/aros/tools/afsplus_clone.c native/aros/client/afsplus_client.c native/aros/client/afsplus_copy.c" \
    "AFSPlusDriverProbe native/aros/tools/afsplus_driver_probe.c"
do
    # shellcheck disable=SC2086 -- the entry is a name and its sources.
    set -- $entry
    program_name=$1
    shift
    program_sources=$*
    # shellcheck disable=SC2086 -- the sources are separate words.
    build_program "$program_name" $program_sources
done

# The probes are test programs, not part of what a person installs, so they
# are built only when a gate asks for them and land beside C/ in Probes/.
# They are built by the same profile as the programs: a probe compiled any
# other way would prove something about a different binary.
if [ "${AFSPLUS_AROS_DIST_PROBES:-0}" = 1 ]; then
    echo "[aros-dist] $profile_id: the probes"
    mkdir -p "$staging/Probes"
    program_dir=$staging/Probes
    build_program AFSPlusAlpha0Probe native/aros/tests/alpha0_probe.c
    build_program AFSPlusDosProbe native/aros/tests/dos_compat_probe.c \
        native/aros/client/afsplus_client.c
    program_dir=$staging/C
fi

# The AROS ELF loader reads a module's symbol table to resolve its
# relocations (rom/dos/internalloadseg_elf.c: relocate() takes sym->shindex
# and sym->value, and the symbol's name only for a debug or error line), and
# it loads the whole of .symtab and .strtab into memory to do it. A local
# symbol no relocation names is read by nobody, so it is discarded: 732
# symbols and 140,784 bytes on aarch64, 738 and 709,512 on x86_64, off the
# package and off what the loader holds while it relocates. The global symbols stay, because the loader's error messages,
# the checks above and genmodule's entry points want them. The audit below
# runs on the stripped file, so what is audited is what ships.
echo "[aros-dist] $profile_id: discard the local symbols"
"$aros_objcopy" --discard-all "$staging/L/afsplus-handler"

echo "[aros-dist] $profile_id: ABI audit"
"$aros_abi_audit" --objdump "$aros_objdump" "$staging/L/afsplus-handler" \
    | tee "$staging/abi-report.txt"

cp native/aros/dist/AFSPLUS.example "$staging/Devs/DOSDrivers/AFSPLUS.example"

if [ "$profile_qualified" = yes ]; then
    cat >"$work/qualification" <<EOF
This build of it runs: the AFS+ gates mount an AFS+ volume through this
handler on Hosted MacAROS, exercise the operation matrix against it, replay
power cuts, and boot a system from an AFS+ partition served by it.
EOF
elif [ "$profile_qualified" = qemu ]; then
    cat >"$work/qualification" <<EOF
This build of it runs. On an AROS $profile_cpu PC in QEMU, booted from the
nightly ISO, it mounts a 64 MiB AFS+ volume on a raw ata.device disk and
serves it: the Alpha-0 operation matrix, the DOS semantics probe and its
hundred steady rounds, the tour of clones, watches and attributes, and a
clean dismount, with the volume checking clean afterwards on the host. What
has not been done on $profile_cpu is a power-cut replay, a boot from an AFS+
partition, and a benchmark.
EOF
else
    cat >"$work/qualification" <<EOF
This build of it has never been run. It is compiled, linked and audited as a
loadable AROS module for $profile_cpu, and that is all this package claims:
nobody has yet mounted a volume with it on an AROS $profile_cpu machine. The
build that has been run is the Hosted MacAROS aarch64 one.
EOF
fi
revision=$(git -C "$repo_root" rev-parse --short HEAD 2>/dev/null || echo unknown)
sed \
    -e "s|@PROFILE_ID@|$profile_id|g" \
    -e "s|@CPU@|$profile_cpu|g" \
    -e "s|@REVISION@|$revision|g" \
    native/aros/dist/ReadMe.template >"$work/ReadMe.head"
awk '
    FNR == NR { text[lines++] = $0; next }
    /@QUALIFICATION@/ { for (i = 0; i < lines; i++) print text[i]; next }
    { print }
' "$work/qualification" "$work/ReadMe.head" >"$staging/ReadMe"

{
    echo "format=afsplus-aros-build-profile-v3"
    echo "profile_id=$profile_id"
    echo "cpu=$profile_cpu"
    echo "qualified=$profile_qualified"
    echo "source_revision=$revision"
    echo "target=$aros_target"
    echo "codegen_target=$aros_codegen_target"
    echo "arch_flags=$aros_arch_flags"
    echo "defines=$aros_defines"
    echo "rust_toolchain=$rust_toolchain"
    echo "rust_target_json=$(basename -- "$aros_rust_target_json")"
    echo "rust_cpu_features=${aros_rust_cpu_features:-none}"
    printf 'rust_target_json_sha256='
    shasum -a 256 "$aros_rust_target_json" | awk '{print $1}'
    printf 'cc_sha256='
    shasum -a 256 "$aros_cc" | awk '{print $1}'
    printf 'collect_aros_sha256='
    shasum -a 256 "$aros_collect_aros" | awk '{print $1}'
    printf 'genmodule_sha256='
    shasum -a 256 "$aros_genmodule" | awk '{print $1}'
    printf 'abi_audit_sha256='
    shasum -a 256 "$aros_abi_audit" | awk '{print $1}'
    printf 'startup_sha256='
    shasum -a 256 "$aros_startup" | awk '{print $1}'
    echo "platform_glue_sha256_begin"
    (
        cd "$aros_platform_glue_dir"
        shasum -a 256 \
            aros_net_glue.c aros_fs_glue.c aros_process_glue.c \
            aros_proc_glue.c aros_env_glue.c
    )
    echo "platform_glue_sha256_end"
} >"$staging/build-profile.txt"

(
    cd "$staging"
    # SHA256SUMS covers every file of the package except itself.
    find . -type f ! -name SHA256SUMS | sed 's|^\./||' | sort \
        | xargs shasum -a 256 >SHA256SUMS
)

# The manifest PUBLISH would sign, printed unsigned. It needs no key, so the
# build machine can produce it and the channel maintainer signs at PUBLISH.
# It is written outside the drawer and moved in, so that it describes the
# package rather than an empty file of its own name.
pkg_tool=${AFSPLUS_PKG:-$(command -v pkg 2>/dev/null || true)}
if [ -n "$pkg_tool" ] && [ -x "$pkg_tool" ]; then
    echo "[aros-dist] $profile_id: Pkg manifest"
    "$pkg_tool" MANIFEST "$staging" NAME afsplus \
        VERSION "${AFSPLUS_PKG_VERSION:-0.1}" ARCH "$profile_cpu" \
        KIND application CONFIG Devs/DOSDrivers/AFSPLUS.example \
        >"$work/MANIFEST.txt" || {
        echo "package-aros-dist: pkg MANIFEST refused this drawer" >&2
        exit 65
    }
    mv "$work/MANIFEST.txt" "$staging/MANIFEST.txt"
else
    echo "[aros-dist] $profile_id: no pkg on PATH, no MANIFEST.txt written"
fi

mkdir -p "$(dirname -- "$output")"
mv "$staging" "$output"
staging=
chmod 755 "$output"

echo "[aros-dist] PASS: $output"
echo "[aros-dist] nothing was installed, started or signed"
