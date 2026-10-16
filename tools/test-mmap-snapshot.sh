#!/bin/sh
# Copyright (C) 2026 John KNIPPER <code@jkn.me>
# Licensed under the AROS Public License (APL) Version 1.1.
# Scoped host proof. Actual AROS scheduler/fault/guest gates are separate.
set -eu
repo=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
[ "$#" = 1 ] || { echo "usage: $0 NEW-OUTPUT" >&2; exit 64; }
output=$1
[ ! -e "$output" ] || { echo "refusing existing output" >&2; exit 73; }
mkdir -p "$output"
output=$(CDPATH= cd -- "$output" && pwd)
: "${AROS_SOURCE:?set AROS_SOURCE to the AROS source tree}"
: "${AFSPLUS_AROS_SDK_ROOT:?set AFSPLUS_AROS_SDK_ROOT to bin/apple-aarch64}"
export CARGO_TARGET_DIR="$output/cargo"
export CARGO_BUILD_JOBS=2
cd "$repo"
cargo test -p afsplus-check --test mmap_snapshot > "$output/core.log" 2>&1
cargo test -p afsplus-aros-ffi --test mmap_snapshot > "$output/ffi.log" 2>&1
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
compile native/aros/tests/mmap_snapshot_packet.c -fsanitize=address,undefined \
    -g -o "$output/packet" > "$output/packet-build.log" 2>&1
"$output/packet" > "$output/packet.log" 2>&1
compile native/aros/tests/packet_stub.c -fsanitize=address,undefined \
    -g -o "$output/regression" > "$output/regression-build.log" 2>&1
"$output/regression" > "$output/regression.log" 2>&1
compile native/aros/tests/mmap_snapshot_ffi_packet.c \
    "$CARGO_TARGET_DIR/debug/libafsplus_aros_ffi.a" \
    -framework Security -framework CoreFoundation -lpthread -ldl \
    -g -o "$output/real-packet" > "$output/real-packet-build.log" 2>&1
"$output/real-packet" > "$output/real-packet.log" 2>&1
printf '%s\n' 'snapshot host core / FFI / packet / real packet PASS (no guest gate)'
