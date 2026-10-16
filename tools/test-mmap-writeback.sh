#!/bin/sh
# Copyright (C) 2026 John KNIPPER <code@jkn.me>
# Licensed under the AROS Public License (APL) Version 1.1.
set -eu
repo=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
[ "$#" = 1 ] || exit 64
output=$1
[ ! -e "$output" ] || exit 73
mkdir -p "$output"
output=$(CDPATH= cd -- "$output" && pwd)
: "${AROS_SOURCE:?set AROS_SOURCE}"
: "${AFSPLUS_AROS_SDK_ROOT:?set AFSPLUS_AROS_SDK_ROOT}"
export CARGO_TARGET_DIR="$output/cargo"
cd "$repo"
cargo build -p afsplus-aros-ffi > "$output/staticlib.log" 2>&1
compile() {
 clang -std=c11 -Wall -Wextra -Werror -nostdlibinc \
  -D__WORDSIZE=64 -DAROS_FAST_BPTR=1 -DAROS_FAST_BSTR=1 \
  -I "$AFSPLUS_AROS_SDK_ROOT/gen/include" \
  -I "$AROS_SOURCE/compiler/crt/stdc/include" \
  -I "$AROS_SOURCE/compiler/crt/stdc/include/aros/stdc" \
  -I "$AROS_SOURCE/compiler/include" -I api -I native/aros \
  native/aros/afsplus_packet.c "$@"
}
compile native/aros/tests/mmap_writeback_ffi_packet.c "$CARGO_TARGET_DIR/debug/libafsplus_aros_ffi.a" \
 -framework Security -framework CoreFoundation -lpthread -ldl -g -o "$output/real-packet" > "$output/real-build.log" 2>&1
"$output/real-packet" > "$output/real.log" 2>&1
compile native/aros/tests/mmap_writeback_packet.c -fsanitize=address,undefined -g \
 -o "$output/writeback-stub" > "$output/writeback-stub-build.log" 2>&1
"$output/writeback-stub" > "$output/writeback-stub.log" 2>&1
compile native/aros/tests/mmap_snapshot_packet.c -fsanitize=address,undefined -g \
 -o "$output/snapshot" > "$output/snapshot-build.log" 2>&1
"$output/snapshot" > "$output/snapshot.log" 2>&1
compile native/aros/tests/packet_stub.c -fsanitize=address,undefined -g \
 -o "$output/regression" > "$output/regression-build.log" 2>&1
"$output/regression" > "$output/regression.log" 2>&1
printf '%s\n' 'writeback packet storage / snapshot / packet regression PASS (no guest/coherence gate)'
