#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause

set -eu

repo=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
work=$(mktemp -d "${TMPDIR:-/tmp}/afsplus-portable-c.XXXXXX")
trap 'rm -R -- "$work"' EXIT HUP INT TERM

image="$work/portable-c.afsp"
intent_image="$work/portable-c-intent.afsp"
writer_image="$work/portable-c-writer.afsp"
writer_low_memory_image="$work/portable-c-writer-low-memory.afsp"
writer_read_fail_image="$work/portable-c-writer-read-fail.afsp"
writer_create_image="$work/portable-c-writer-create.afsp"
writer_create_low_memory_image="$work/portable-c-writer-create-low-memory.afsp"
writer_create_exists_image="$work/portable-c-writer-create-exists.afsp"
writer_create_exhausted_image="$work/portable-c-writer-create-exhausted.afsp"
writer_create_bad_watermark_image="$work/portable-c-writer-create-bad-watermark.afsp"
writer_create_missing_parent_image="$work/portable-c-writer-create-missing-parent.afsp"
writer_torn_image="$work/portable-c-writer-torn.afsp"
writer_torn_only_image="$work/portable-c-writer-torn-only.afsp"
writer_flush_image="$work/portable-c-writer-flush.afsp"
writer_exists_image="$work/portable-c-writer-exists.afsp"
writer_delete_image="$work/portable-c-writer-delete.afsp"
writer_replace_image="$work/portable-c-writer-replace.afsp"
sanitized_writer_image="$work/portable-c-writer-sanitized.afsp"
intent_expected="$work/intent-expected.bin"
intent_created_expected="$work/intent-created-expected.bin"
source_tree="$work/source"
probe="$work/reader-probe"
intent_probe="$work/intent-probe"
writer_probe="$work/writer-probe"
sanitized_probe="$work/reader-probe-sanitized"
sanitized_intent_probe="$work/intent-probe-sanitized"
sanitized_writer_probe="$work/writer-probe-sanitized"
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
    -p afsplus-tools --bin mkafsplus -- \
    --profile classic-rw --size-mib 16 --label PortableC "$image"
cargo run --quiet --manifest-path "$repo/Cargo.toml" \
    -p afsplus-core --bin afsplus-populate -- "$image" "$source_tree"
cargo run --quiet --manifest-path "$repo/Cargo.toml" \
    -p afsplus-check --bin afsplus-portable-c-log-fixture -- \
    "$intent_image" "$intent_expected" "$intent_created_expected"
cp "$intent_image" "$writer_image"
cp "$intent_image" "$writer_low_memory_image"
cp "$intent_image" "$writer_read_fail_image"
cp "$intent_image" "$writer_create_image"
cp "$intent_image" "$writer_create_low_memory_image"
cp "$intent_image" "$writer_create_exists_image"
cp "$image" "$writer_create_exhausted_image"
cp "$image" "$writer_create_bad_watermark_image"
cp "$intent_image" "$writer_create_missing_parent_image"
cp "$intent_image" "$writer_torn_image"
cp "$intent_image" "$writer_torn_only_image"
cp "$intent_image" "$writer_flush_image"
cp "$intent_image" "$writer_exists_image"
cp "$intent_image" "$writer_delete_image"
cp "$intent_image" "$writer_replace_image"
cp "$intent_image" "$sanitized_writer_image"
cargo run --quiet --manifest-path "$repo/Cargo.toml" \
    -p afsplus-check --bin afsplus-portable-c-log-fixture -- \
    --exhaust-object-ids "$writer_create_exhausted_image"
cargo run --quiet --manifest-path "$repo/Cargo.toml" \
    -p afsplus-check --bin afsplus-portable-c-log-fixture -- \
    --regress-object-watermark "$writer_create_bad_watermark_image"

"$compiler" -std=c99 -pedantic -Wall -Wextra -Werror -Wconversion \
    -Wshadow -Wstrict-prototypes \
    -I"$repo/api" -I"$repo/spec" \
    "$repo/portable/c/reader.c" "$repo/portable/c/tests/reader_probe.c" \
    -o "$probe"
"$probe" "$image" "$repo/README.md"

"$compiler" -std=c99 -pedantic -Wall -Wextra -Werror -Wconversion \
    -Wshadow -Wstrict-prototypes \
    -I"$repo/api" -I"$repo/spec" \
    "$repo/portable/c/reader.c" "$repo/portable/c/tests/intent_probe.c" \
    -o "$intent_probe"
"$intent_probe" "$intent_image" "$intent_expected" \
    "$intent_created_expected"

"$compiler" -std=c99 -pedantic -Wall -Wextra -Werror -Wconversion \
    -Wshadow -Wstrict-prototypes \
    -I"$repo/api" -I"$repo/spec" \
    "$repo/portable/c/reader.c" "$repo/portable/c/writer.c" \
    "$repo/portable/c/tests/writer_probe.c" -o "$writer_probe"
"$writer_probe" "$writer_image"
"$writer_probe" "$writer_low_memory_image" low-memory
"$writer_probe" "$writer_read_fail_image" read-fail-retry
"$writer_probe" "$writer_create_image" create
"$writer_probe" "$writer_create_low_memory_image" create-low-memory
"$writer_probe" "$writer_create_exists_image" create-exists
"$writer_probe" "$writer_create_exhausted_image" create-exhausted
"$writer_probe" "$writer_create_bad_watermark_image" create-bad-watermark
"$writer_probe" "$writer_create_missing_parent_image" create-missing-parent
"$writer_probe" "$writer_torn_image" torn-retry
"$writer_probe" "$writer_torn_only_image" torn-only
"$writer_probe" "$writer_flush_image" flush-fail
"$writer_probe" "$writer_exists_image" destination-exists
"$writer_probe" "$writer_delete_image" delete
"$writer_probe" "$writer_replace_image" replace
cargo run --quiet --manifest-path "$repo/Cargo.toml" \
    -p afsplus-check --bin afsplus-portable-c-log-fixture -- \
    --verify-c-rename "$writer_image" "$intent_expected" \
    "$intent_created_expected"
cargo run --quiet --manifest-path "$repo/Cargo.toml" \
    -p afsplus-check --bin afsplus-portable-c-log-fixture -- \
    --verify-c-rename "$writer_low_memory_image" "$intent_expected" \
    "$intent_created_expected"
cargo run --quiet --manifest-path "$repo/Cargo.toml" \
    -p afsplus-check --bin afsplus-portable-c-log-fixture -- \
    --verify-c-rename "$writer_read_fail_image" "$intent_expected" \
    "$intent_created_expected"
cargo run --quiet --manifest-path "$repo/Cargo.toml" \
    -p afsplus-check --bin afsplus-portable-c-log-fixture -- \
    --verify-c-create "$writer_create_image" "$intent_expected" \
    "$intent_created_expected"
cargo run --quiet --manifest-path "$repo/Cargo.toml" \
    -p afsplus-check --bin afsplus-portable-c-log-fixture -- \
    --verify-c-create "$writer_create_low_memory_image" "$intent_expected" \
    "$intent_created_expected"
cargo run --quiet --manifest-path "$repo/Cargo.toml" \
    -p afsplus-check --bin afsplus-portable-c-log-fixture -- \
    --verify-c-rename "$writer_torn_image" "$intent_expected" \
    "$intent_created_expected"
cargo run --quiet --manifest-path "$repo/Cargo.toml" \
    -p afsplus-check --bin afsplus-check -- "$writer_torn_only_image" >/dev/null
cargo run --quiet --manifest-path "$repo/Cargo.toml" \
    -p afsplus-check --bin afsplus-portable-c-log-fixture -- \
    --verify-c-torn "$writer_torn_only_image" "$intent_expected" \
    "$intent_created_expected"
cargo run --quiet --manifest-path "$repo/Cargo.toml" \
    -p afsplus-check --bin afsplus-portable-c-log-fixture -- \
    --verify-c-delete "$writer_delete_image" "$intent_expected" \
    "$intent_created_expected"
cargo run --quiet --manifest-path "$repo/Cargo.toml" \
    -p afsplus-check --bin afsplus-portable-c-log-fixture -- \
    --verify-c-replace "$writer_replace_image" "$intent_expected" \
    "$intent_created_expected"

"$compiler" -std=c99 -pedantic -Wall -Wextra -Werror -Wconversion \
    -Wshadow -Wstrict-prototypes \
    -I"$repo/api" -I"$repo/spec" \
    "$repo/portable/c/reader.c" "$repo/portable/c/examples/probe_file.c" \
    -o "$example"
"$example" "$image"

if command -v c++ >/dev/null 2>&1; then
    c++ -std=c++11 -pedantic -Wall -Wextra -Werror \
        -I"$repo/api" -include libafsplus_reader.h \
        -include libafsplus_writer.h \
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

if "$compiler" -std=c99 -g -fno-omit-frame-pointer \
    -fsanitize=address,undefined \
    -I"$repo/api" -I"$repo/spec" \
    "$repo/portable/c/reader.c" "$repo/portable/c/tests/intent_probe.c" \
    -o "$sanitized_intent_probe" 2>/dev/null; then
    ASAN_OPTIONS=halt_on_error=1 \
        UBSAN_OPTIONS=halt_on_error=1 \
        "$sanitized_intent_probe" "$intent_image" "$intent_expected" \
        "$intent_created_expected"
else
    echo "portable-c-intent sanitizers=SKIP compiler=$compiler"
fi

if "$compiler" -std=c99 -g -fno-omit-frame-pointer \
    -fsanitize=address,undefined \
    -I"$repo/api" -I"$repo/spec" \
    "$repo/portable/c/reader.c" "$repo/portable/c/writer.c" \
    "$repo/portable/c/tests/writer_probe.c" \
    -o "$sanitized_writer_probe" 2>/dev/null; then
    ASAN_OPTIONS=halt_on_error=1 \
        UBSAN_OPTIONS=halt_on_error=1 \
        "$sanitized_writer_probe" "$sanitized_writer_image"
else
    echo "portable-c-writer sanitizers=SKIP compiler=$compiler"
fi

if "$compiler" --version 2>/dev/null | grep -qi clang; then
    "$compiler" --analyze -std=c99 -I"$repo/api" -I"$repo/spec" \
        "$repo/portable/c/reader.c" -o /dev/null
    "$compiler" --analyze -std=c99 -I"$repo/api" -I"$repo/spec" \
        "$repo/portable/c/tests/intent_probe.c" -o /dev/null
    "$compiler" --analyze -std=c99 -I"$repo/api" -I"$repo/spec" \
        "$repo/portable/c/writer.c" -o /dev/null
    "$compiler" --analyze -std=c99 -I"$repo/api" -I"$repo/spec" \
        "$repo/portable/c/tests/writer_probe.c" -o /dev/null
    echo "portable-c-reader static-analyzer=PASS"
else
    echo "portable-c-reader static-analyzer=SKIP non-clang-compiler"
fi

m68k_compiler=${AFSPLUS_M68K_CC:-"$HOME/aros-m68k-build/bin/darwin-aarch64/tools/crosstools/m68k-aros-gcc"}
if [ -x "$m68k_compiler" ]; then
    "$m68k_compiler" -std=c99 -Wall -Wextra -Werror \
        -I"$repo/api" -I"$repo/spec" \
        -c "$repo/portable/c/reader.c" -o "$work/reader-m68k.o"
    "$m68k_compiler" -std=c99 -Wall -Wextra -Werror -fstack-usage \
        -I"$repo/api" -I"$repo/spec" \
        -c "$repo/portable/c/writer.c" -o "$work/writer-m68k.o"
    writer_stack=$(awk -F '\t' \
        '$1 ~ /afspw_append_namespace$/ { print $2; found = 1 } \
         END { if (!found) exit 1 }' "$work/writer-m68k.su")
    test "$writer_stack" -le 1024
    echo "portable-c-reader m68k-compile=PASS writer-frame=$writer_stack ceiling=1024"
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
    test -f "$work/install/include/libafsplus_writer.h"
    test -f "$work/install/lib/libafsplus_reader.a"
    test -f "$work/install/lib/libafsplus_writer.a"
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
cargo run --quiet --manifest-path "$repo/Cargo.toml" \
    -p afsplus-check --bin afsplus-check -- "$intent_image" >/dev/null
cargo run --quiet --manifest-path "$repo/Cargo.toml" \
    -p afsplus-check --bin afsplus-check -- "$writer_image" >/dev/null
cargo run --quiet --manifest-path "$repo/Cargo.toml" \
    -p afsplus-check --bin afsplus-check -- "$writer_low_memory_image" >/dev/null
cargo run --quiet --manifest-path "$repo/Cargo.toml" \
    -p afsplus-check --bin afsplus-check -- "$writer_read_fail_image" >/dev/null
cargo run --quiet --manifest-path "$repo/Cargo.toml" \
    -p afsplus-check --bin afsplus-check -- "$writer_create_image" >/dev/null
cargo run --quiet --manifest-path "$repo/Cargo.toml" \
    -p afsplus-check --bin afsplus-check -- "$writer_create_low_memory_image" >/dev/null
cargo run --quiet --manifest-path "$repo/Cargo.toml" \
    -p afsplus-check --bin afsplus-check -- "$writer_create_exists_image" >/dev/null
cargo run --quiet --manifest-path "$repo/Cargo.toml" \
    -p afsplus-check --bin afsplus-check -- "$writer_create_exhausted_image" >/dev/null
cargo run --quiet --manifest-path "$repo/Cargo.toml" \
    -p afsplus-check --bin afsplus-check -- "$writer_create_missing_parent_image" >/dev/null
cargo run --quiet --manifest-path "$repo/Cargo.toml" \
    -p afsplus-check --bin afsplus-check -- "$writer_torn_image" >/dev/null
cargo run --quiet --manifest-path "$repo/Cargo.toml" \
    -p afsplus-check --bin afsplus-check -- "$writer_torn_only_image" >/dev/null
cargo run --quiet --manifest-path "$repo/Cargo.toml" \
    -p afsplus-check --bin afsplus-check -- "$writer_flush_image" >/dev/null
cargo run --quiet --manifest-path "$repo/Cargo.toml" \
    -p afsplus-check --bin afsplus-check -- "$writer_exists_image" >/dev/null
cargo run --quiet --manifest-path "$repo/Cargo.toml" \
    -p afsplus-check --bin afsplus-check -- "$writer_delete_image" >/dev/null
cargo run --quiet --manifest-path "$repo/Cargo.toml" \
    -p afsplus-check --bin afsplus-check -- "$writer_replace_image" >/dev/null

echo "portable-c-gate result=PASS reader=PASS intent-view=PASS writer-create=PASS writer-rename=PASS writer-delete=PASS writer-replace=PASS"
