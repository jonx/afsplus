#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause
#
# Development check: compile and run the host DosPacket and trackdisk
# matrices against the AROS *source* headers, for a host that has the AROS
# source tree but no built SDK. The generated aros/config.h is synthesised
# from config.h.in, so this is never qualification evidence; the gate is
# tools/check-aros-ffi.sh with a built SDK.

set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
aros_source=${AROS_SOURCE:-"$repo_root/../aros-apple-core"}
if [ ! -d "$aros_source" ]; then
    # A lot worktree lives outside the source tree; use the main checkout's
    # sibling.
    common=$(git -C "$repo_root" rev-parse --path-format=absolute \
        --git-common-dir)
    aros_source="$common/../../aros-apple-core"
fi
work=${AFSPLUS_DEV_PACKET_WORK:-"$repo_root/target/dev-packet-matrix"}

for required in \
    "$aros_source/config/config.h.in" \
    "$aros_source/compiler/include/dos/dos64.h" \
    "$aros_source/compiler/arossupport/include/system.h" \
    "$aros_source/arch/aarch64-all/include/aros/cpu.h" \
    "$aros_source/compiler/crt/stdc/include/aros/stdc/string.h"; do
    if [ ! -f "$required" ]; then
        echo "[dev-packet] missing $required" >&2
        exit 2
    fi
done

rm -rf "$work"
mkdir -p "$work/include/aros/aarch64"
for header in "$aros_source"/compiler/arossupport/include/*.h; do
    ln -s "$header" "$work/include/aros/"
done
for header in "$aros_source"/arch/aarch64-all/include/aros/*.h; do
    ln -s "$header" "$work/include/aros/aarch64/"
done
sed -e 's/@aros_amigaos_compliance@/0/' \
    -e 's/@aros_flavour_uc@/(AROS_FLAVOUR_STANDALONE)/' \
    -e 's/@aros_nominal_width@/800/' \
    -e 's/@aros_nominal_height@/600/' \
    -e 's/@aros_nominal_depth@/4/' \
    -e 's/@aros_nesting_supervisor@/1/' \
    -e '/^@[A-Za-z_]*@$/d' \
    -e 's/@[A-Za-z_]*@/0/g' \
    "$aros_source/config/config.h.in" > "$work/include/aros/config.h"

stdc="$aros_source/compiler/crt/stdc/include"
cd "$repo_root"

run_matrix() {
    name=$1
    shift
    clang -std=c11 -Wall -Wextra -Werror -nostdlibinc \
        -D__WORDSIZE=64 -DAROS_FAST_BPTR=1 -DAROS_FAST_BSTR=1 \
        -I "$work/include" -I "$stdc" -I "$stdc/aros/stdc" \
        -I "$aros_source/compiler/include" -I api -I native/aros \
        "$@" -o "$work/$name"
    "$work/$name"
}

echo "[dev-packet] DosPacket translation matrix (source headers)"
run_matrix packet-stub native/aros/afsplus_packet.c \
    native/aros/tests/packet_stub.c
echo "[dev-packet] trackdisk viewport matrix (source headers)"
run_matrix trackdisk-stub native/aros/afsplus_trackdisk.c \
    native/aros/tests/trackdisk_stub.c
# The handler shell calls Exec and DOS, so it needs the installed SDK include
# tree of a running AROS build. Its generated proto headers arrive late in
# that build; native/aros/tests/dev-proto stands in for the three the shell
# uses. dos/dos64.h comes from the SDK when it is there, else from the AROS
# source tree. This is a syntax and type check only.
sdk=${AFSPLUS_AROS_SDK_ROOT:-"$HOME/aros-build/bin/darwin-aarch64"}
if [ -f "$sdk/AROS/Developer/include/exec/execbase.h" ] \
    && [ -f "$sdk/gen/include/aros/config.h" ]; then
    echo "[dev-packet] handler shell type check (SDK include tree, stand-in proto headers)"
    mkdir -p "$work/dos64/dos"
    if [ -f "$sdk/AROS/Developer/include/dos/dos64.h" ]; then
        ln -sf "$sdk/AROS/Developer/include/dos/dos64.h" "$work/dos64/dos/dos64.h"
    else
        ln -sf "$aros_source/compiler/include/dos/dos64.h" "$work/dos64/dos/dos64.h"
    fi
    clang -std=gnu11 -fsyntax-only -Wall -Wextra -Werror \
        -Wno-unused-variable -Wno-unused-but-set-variable \
        -Wno-unused-parameter -nostdlibinc -D__WORDSIZE=64 \
        -I native/aros/tests/dev-proto \
        -I "$sdk/AROS/Developer/include/aros/stdc" \
        -I "$sdk/AROS/Developer/include" -I "$sdk/gen/include" \
        -I "$work/dos64" -I api -I native/aros \
        native/aros/afsplus_handler.c
else
    echo "[dev-packet] handler shell type check skipped: no SDK include tree at $sdk"
fi
echo "[dev-packet] PASS (development check, no qualification claim)"
