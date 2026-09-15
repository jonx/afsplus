#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause

set -eu

repo=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
work=$(mktemp -d "${TMPDIR:-/tmp}/afsplus-rust-codec-fuzz.XXXXXX")
trap 'rm -R -- "$work"' EXIT HUP INT TERM

runs=${AFSPLUS_RUST_FUZZ_RUNS:-4096}
target_dir="$repo/target/rust-codec-fuzz"
manifest="$repo/fuzz/Cargo.toml"
progress="$work/current-case.txt"
failure=${AFSPLUS_RUST_FUZZ_ARTIFACT:-"$repo/build/rust-codec-fuzz-failure.afrf"}

case "$runs" in
    ''|*[!0-9]*)
        echo "AFSPLUS_RUST_FUZZ_RUNS must be a positive integer" >&2
        exit 2
        ;;
    0)
        echo "AFSPLUS_RUST_FUZZ_RUNS must not be zero" >&2
        exit 2
        ;;
esac

export CARGO_TARGET_DIR="$target_dir"
mkdir -p "$(dirname -- "$failure")"

cargo fmt --manifest-path "$manifest" -- --check
cargo test --quiet --manifest-path "$manifest"
cargo clippy --quiet --manifest-path "$manifest" --all-targets -- -D warnings

if ! cargo run --quiet --release --manifest-path "$manifest" -- \
    --runs "$runs" --progress "$progress" --artifact "$failure"; then
    echo "rust-codec-fuzz failure; last case:" >&2
    sed -n '1,20p' "$progress" >&2
    if [ -f "$failure" ]; then
        echo "artifact=$failure" >&2
        echo "replay with: cargo run --manifest-path fuzz/Cargo.toml -- --replay $failure" >&2
    fi
    exit 1
fi

for target in tree-node bitmap-page region-descriptor snapshot-registry snapshot-record snapshot-lifetime snapshot-ledger snapshot-key reclaim-root reclaim-segment reclaim-table snapshot-checkpoint inline-symlink object-metadata legacy-directory legacy-object-map legacy-retired; do
    replay="$work/$target.afrf"
    cargo run --quiet --release --manifest-path "$manifest" -- \
        --target "$target" --case 47 --artifact "$replay"
    cargo run --quiet --release --manifest-path "$manifest" -- --replay "$replay"
done

for regression in "$repo"/fuzz/regressions/*.afrf; do
    [ -e "$regression" ] || continue
    cargo run --quiet --release --manifest-path "$manifest" -- --replay "$regression"
done

echo "rust-codec-fuzz gate=PASS runs-per-target=$runs"
