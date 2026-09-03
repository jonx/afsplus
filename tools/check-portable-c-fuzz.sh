#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause

set -eu

repo=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
work=$(mktemp -d "${TMPDIR:-/tmp}/afsplus-portable-c-fuzz.XXXXXX")
trap 'rm -R -- "$work"' EXIT HUP INT TERM

image="$work/portable-c-fuzz.afsp"
intent_image="$work/portable-c-fuzz-intent.afsp"
intent_expected="$work/intent-expected.bin"
intent_created="$work/intent-created.bin"
source_tree="$work/source"
corpus="$work/corpus"
packer="$work/afsplus-fuzz-pack"
replay="$work/afsplus-fuzz-replay"
smoke="$work/afsplus-fuzz-smoke"
sanitized_smoke="$work/afsplus-fuzz-smoke-sanitized"
progress="$work/current-case.txt"
compiler=${CC:-cc}
runs=${AFSPLUS_FUZZ_RUNS:-4096}

case "$runs" in
    ''|*[!0-9]*)
        echo "AFSPLUS_FUZZ_RUNS must be a positive integer" >&2
        exit 2
        ;;
    0)
        echo "AFSPLUS_FUZZ_RUNS must not be zero" >&2
        exit 2
        ;;
esac

mkdir "$source_tree" "$corpus"
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
    --profile classic-rw --size-mib 16 --label PortableFuzz "$image"
cargo run --quiet --manifest-path "$repo/Cargo.toml" \
    -p afsplus-core --bin afsplus-populate -- "$image" "$source_tree"
cargo run --quiet --manifest-path "$repo/Cargo.toml" \
    -p afsplus-check --bin afsplus-portable-c-log-fixture -- \
    "$intent_image" "$intent_expected" "$intent_created"

cflags="-std=c99 -pedantic -Wall -Wextra -Werror -Wconversion -Wshadow -Wstrict-prototypes"
includes="-I$repo/api -I$repo/spec -I$repo/portable/c/fuzz"

# shellcheck disable=SC2086
"$compiler" $cflags $includes \
    "$repo/portable/c/reader.c" "$repo/portable/c/fuzz/harness.c" \
    "$repo/portable/c/fuzz/seed_pack.c" -o "$packer"
# shellcheck disable=SC2086
"$compiler" $cflags $includes \
    "$repo/portable/c/reader.c" "$repo/portable/c/fuzz/harness.c" \
    "$repo/portable/c/fuzz/replay.c" -o "$replay"
# shellcheck disable=SC2086
"$compiler" $cflags $includes \
    "$repo/portable/c/reader.c" "$repo/portable/c/fuzz/harness.c" \
    "$repo/portable/c/fuzz/fuzz_target.c" \
    "$repo/portable/c/fuzz/smoke.c" -o "$smoke"

"$packer" "$image" "$corpus/00-probe.afzf" 0 0 0 0
"$packer" "$image" "$corpus/01-root.afzf" 1 1 0 0
"$packer" "$image" "$corpus/02-directory-first.afzf" 2 0 0 0
"$packer" "$image" "$corpus/03-directory-last.afzf" 2 302 0 0
"$packer" "$image" "$corpus/04-directory-file.afzf" 4 302 0 777
"$packer" "$intent_image" "$corpus/05-intent-scan.afzf" 5 0 0 0
"$packer" "$intent_image" "$corpus/06-intent-namespace.afzf" 6 1 0 0

for seed in "$corpus"/*.afzf; do
    "$replay" -s "$seed"
done

if "$compiler" -std=c99 -g -fno-omit-frame-pointer \
    -fsanitize=address,undefined $includes \
    "$repo/portable/c/reader.c" "$repo/portable/c/fuzz/harness.c" \
    "$repo/portable/c/fuzz/fuzz_target.c" \
    "$repo/portable/c/fuzz/smoke.c" -o "$sanitized_smoke" 2>/dev/null; then
    if ! ASAN_OPTIONS=halt_on_error=1 UBSAN_OPTIONS=halt_on_error=1 \
        "$sanitized_smoke" --runs "$runs" --progress "$progress" \
        "$corpus"/*.afzf; then
        echo "portable-c-fuzz sanitizer failure; reproduce with:" >&2
        cat "$progress" >&2
        echo "  $sanitized_smoke --case CASE --artifact failure.afzf SEED" >&2
        exit 1
    fi
else
    echo "portable-c-fuzz sanitizers=SKIP compiler=$compiler"
    "$smoke" --runs "$runs" --progress "$progress" "$corpus"/*.afzf
fi

if "$compiler" --version 2>/dev/null | grep -qi clang; then
    for source in harness.c fuzz_target.c smoke.c replay.c seed_pack.c; do
        "$compiler" --analyze -Werror -std=c99 $includes \
            "$repo/portable/c/fuzz/$source" -o /dev/null
    done
    echo "portable-c-fuzz static-analyzer=PASS"
else
    echo "portable-c-fuzz static-analyzer=SKIP non-clang-compiler"
fi

"$smoke" --case 47 --artifact "$work/replay-case.afzf" \
    "$corpus/04-directory-file.afzf"
"$replay" "$work/replay-case.afzf"

# The callback is directly compatible with libFuzzer. Apple command-line tools
# sometimes omit the runtime, so the deterministic sanitizer runner above is
# the mandatory gate and native libFuzzer is an opportunistic second engine.
if "$compiler" -std=c99 -g -fno-omit-frame-pointer \
    -fsanitize=fuzzer,address,undefined $includes \
    "$repo/portable/c/reader.c" "$repo/portable/c/fuzz/harness.c" \
    "$repo/portable/c/fuzz/fuzz_target.c" -o "$work/afsplus-libfuzzer" \
    2>/dev/null; then
    ASAN_OPTIONS=halt_on_error=1 UBSAN_OPTIONS=halt_on_error=1 \
        "$work/afsplus-libfuzzer" -runs="$runs" "$corpus" >/dev/null
    echo "portable-c-fuzz libfuzzer=PASS runs=$runs"
else
    echo "portable-c-fuzz libfuzzer=SKIP runtime-not-found"
fi

if command -v cmake >/dev/null 2>&1; then
    cmake -S "$repo/portable/c" -B "$work/cmake" \
        -DAFSPLUS_READER_BUILD_EXAMPLE=OFF \
        -DAFSPLUS_READER_BUILD_FUZZ_TOOLS=ON >/dev/null
    cmake --build "$work/cmake" >/dev/null
    "$work/cmake/afsplus-fuzz-replay" -s "$corpus/04-directory-file.afzf"
else
    echo "portable-c-fuzz cmake=SKIP command-not-found"
fi

cargo run --quiet --manifest-path "$repo/Cargo.toml" \
    -p afsplus-check --bin afsplus-check -- "$image" >/dev/null
echo "portable-c-fuzz result=PASS runs-per-seed=$runs seeds=7"
