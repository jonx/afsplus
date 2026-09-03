#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause

set -eu

repo=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
work=$(mktemp -d "${TMPDIR:-/tmp}/afsplus-portable-c.XXXXXX")
trap 'rm -R -- "$work"' EXIT HUP INT TERM

image="$work/portable-c.afsp"
source_tree="$work/source"
probe="$work/reader-probe"
sanitized_probe="$work/reader-probe-sanitized"
example="$work/afsplus-reader-probe"
compiler=${CC:-cc}

mkdir "$source_tree"
cp "$repo/README.md" "$source_tree/readme.md"
cp "$repo/LICENSE.md" "$source_tree/license.md"
touch "$source_tree/Café"
fixture_index=0
while [ "$fixture_index" -lt 300 ]; do
    touch "$source_tree/item-$fixture_index"
    fixture_index=$((fixture_index + 1))
done

cargo run --quiet --manifest-path "$repo/Cargo.toml" \
    -p afsplus-core --bin afsplus-mkfs -- \
    --size-mib 16 --label PortableC "$image"
cargo run --quiet --manifest-path "$repo/Cargo.toml" \
    -p afsplus-core --bin afsplus-populate -- "$image" "$source_tree"

"$compiler" -std=c99 -pedantic -Wall -Wextra -Werror -Wconversion \
    -Wshadow -Wstrict-prototypes \
    -I"$repo/api" -I"$repo/spec" \
    "$repo/portable/c/reader.c" "$repo/portable/c/tests/reader_probe.c" \
    -o "$probe"
"$probe" "$image" "$repo/README.md"

"$compiler" -std=c99 -pedantic -Wall -Wextra -Werror -Wconversion \
    -Wshadow -Wstrict-prototypes \
    -I"$repo/api" -I"$repo/spec" \
    "$repo/portable/c/reader.c" "$repo/portable/c/examples/probe_file.c" \
    -o "$example"
"$example" "$image"

if command -v c++ >/dev/null 2>&1; then
    c++ -std=c++11 -pedantic -Wall -Wextra -Werror \
        -I"$repo/api" -include libafsplus_reader.h \
        -fsyntax-only -x c++ /dev/null
else
    echo "portable-c-reader cxx-header=SKIP compiler-not-found"
fi

if "$compiler" -std=c99 -g -fno-omit-frame-pointer \
    -fsanitize=address,undefined \
    -I"$repo/api" -I"$repo/spec" \
    "$repo/portable/c/reader.c" "$repo/portable/c/tests/reader_probe.c" \
    -o "$sanitized_probe" 2>/dev/null; then
    ASAN_OPTIONS=halt_on_error=1 \
        UBSAN_OPTIONS=halt_on_error=1 \
        "$sanitized_probe" "$image" "$repo/README.md"
else
    echo "portable-c-reader sanitizers=SKIP compiler=$compiler"
fi

if "$compiler" --version 2>/dev/null | grep -qi clang; then
    "$compiler" --analyze -std=c99 -I"$repo/api" -I"$repo/spec" \
        "$repo/portable/c/reader.c" -o /dev/null
    echo "portable-c-reader static-analyzer=PASS"
else
    echo "portable-c-reader static-analyzer=SKIP non-clang-compiler"
fi

m68k_compiler=${AFSPLUS_M68K_CC:-"$HOME/aros-m68k-build/bin/darwin-aarch64/tools/crosstools/m68k-aros-gcc"}
if [ -x "$m68k_compiler" ]; then
    "$m68k_compiler" -std=c99 -Wall -Wextra -Werror \
        -I"$repo/api" -I"$repo/spec" \
        -c "$repo/portable/c/reader.c" -o "$work/reader-m68k.o"
    echo "portable-c-reader m68k-compile=PASS"
else
    echo "portable-c-reader m68k-compile=SKIP compiler-not-found"
fi

if command -v cmake >/dev/null 2>&1; then
    cmake -S "$repo/portable/c" -B "$work/cmake" \
        -DAFSPLUS_READER_BUILD_EXAMPLE=ON >/dev/null
    cmake --build "$work/cmake" >/dev/null
    "$work/cmake/afsplus-reader-probe" "$image"
    cmake --install "$work/cmake" --prefix "$work/install" >/dev/null
    test -f "$work/install/include/libafsplus_reader.h"
    test -f "$work/install/lib/libafsplus_reader.a"
    test -f "$work/install/lib/cmake/AFSPlusReader/AFSPlusReaderConfig.cmake"
    cmake -S "$repo/portable/c/tests/cmake_consumer" \
        -B "$work/consumer" -DCMAKE_PREFIX_PATH="$work/install" >/dev/null
    cmake --build "$work/consumer" >/dev/null
    "$work/consumer/afsplus-reader-consumer"
else
    echo "portable-c-reader cmake=SKIP command-not-found"
fi

cargo run --quiet --manifest-path "$repo/Cargo.toml" \
    -p afsplus-check --bin afsplus-check -- "$image" >/dev/null
