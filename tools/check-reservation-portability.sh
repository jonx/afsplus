#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause
set -eu
repo=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
work=$(mktemp -d "${TMPDIR:-/tmp}/afsplus-reservation-c.XXXXXX")
trap 'rm -R -- "$work"' EXIT HUP INT TERM
cargo run --quiet --manifest-path "$repo/Cargo.toml" -p afsplus-check --bin afsplus-reservation-fixture -- "$work/images"
compiler=${CC:-cc}
printf 'reservation_portability host=%s compiler=%s\n' "$(uname -m)" "$compiler"
for mode in strict sanitized; do
    flags=""
    if [ "$mode" = sanitized ]; then flags="-fsanitize=address,undefined -fno-omit-frame-pointer"; fi
    # Intentional splitting of this fixed local compiler flag list.
    "$compiler" -std=c99 -pedantic -Wall -Wextra -Werror -Wconversion -Wshadow -Wstrict-prototypes $flags \
        -I"$repo/api" -I"$repo/spec" "$repo/portable/c/reader.c" \
        "$repo/portable/c/tests/reservation_probe.c" -o "$work/probe-$mode"
    for image in before after fallback; do
        "$work/probe-$mode" "$work/images/$image.afsp" "$image"
    done
    if "$work/probe-$mode" "$work/images/after.afsp" before >"$work/negative-$mode.log" 2>&1; then
        echo "reservation negative control incorrectly accepted initialized data as zeros" >&2
        exit 1
    fi
    grep -q 'logical byte mismatch' "$work/negative-$mode.log"
    printf 'reservation_c build=%s negative-control=PASS\n' "$mode"
done
m68k_compiler=${AFSPLUS_M68K_CC:-"$HOME/aros-m68k-build/bin/darwin-aarch64/tools/crosstools/m68k-aros-gcc"}
if [ -x "$m68k_compiler" ]; then
    "$m68k_compiler" -m68000 -std=c99 -Wall -Wextra -Werror -fstack-usage \
        -I"$repo/api" -I"$repo/spec" -c "$repo/portable/c/reader.c" -o "$work/reader-m68000.o"
    "$m68k_compiler" -m68000 -std=c99 -Wall -Wextra -Werror \
        -I"$repo/api" -I"$repo/spec" -c "$repo/portable/c/tests/reservation_probe.c" -o "$work/reservation-m68000.o"
    frame=$(awk 'BEGIN { max=0; found=0 } { if ($2+0>max) max=$2+0; found=1 } END { if (!found) exit 1; print max }' "$work/reader-m68000.su")
    printf 'reservation_c m68000-compile=PASS reader-max-frame=%s runtime=UNVERIFIED\n' "$frame"
else
    echo 'reservation_c m68000-compile=SKIP compiler-not-found runtime=UNVERIFIED'
fi
