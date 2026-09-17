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
echo "[dev-packet] PASS (development check, no qualification claim)"
