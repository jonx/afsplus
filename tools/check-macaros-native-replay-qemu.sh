#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause

# Replay every deterministic intent-log cut under native MacAROS QEMU, then
# extract and strictly check the resulting AFS+ payload on the host.

set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
output=${AFSPLUS_MACAROS_REPLAY_OUTPUT:-"$repo_root/build/macaros-native-replay-qemu"}
work=$(mktemp -d /tmp/afsplus-nreplay.XXXXXX)
fixtures="$work/fixtures"
result="$work/result"

cleanup() {
    status=$?
    if [ "$status" -ne 0 ] && [ "${AFSPLUS_KEEP_NATIVE_REPLAY_FAILURE:-0}" = 1 ]; then
        echo "[native-replay] keeping failed work directory: $work" >&2
        return
    fi
    [ ! -d "$work" ] || rm -r "$work"
}
trap cleanup EXIT HUP INT TERM

[ ! -e "$output" ] || {
    echo "Refusing to replace existing result: $output" >&2
    exit 73
}

mkdir -p "$result/cases"
cd "$repo_root"

echo "[native-replay] generate deterministic power-cut images"
cargo run --quiet --release -p afsplus-check \
    --bin afsplus-crash-fixtures -- "$fixtures"
cp "$fixtures/manifest.tsv" "$fixtures/README.txt" "$result/"

tab=$(printf '\t')
while IFS="$tab" read -r fixture expected pending_before description; do
    [ "$fixture" != fixture ] || continue
    case_name=${fixture%.img}
    case_output="$result/cases/$case_name"
    check_before="$work/$case_name-check-before.json"

    echo "[native-replay] $case_name: expect $expected ($description)"
    cargo run --quiet --release -p afsplus-check --bin afsplus-check -- \
        "$fixtures/$fixture" --json >"$check_before"
    grep -q '"clean":true' "$check_before"
    grep -q "\"log_records_pending\":$pending_before" "$check_before"

    AFSPLUS_MACAROS_QEMU_MODE="replay-$expected" \
    AFSPLUS_MACAROS_QEMU_AFSPLUS_IMAGE="$fixtures/$fixture" \
    AFSPLUS_MACAROS_QEMU_OUTPUT="$case_output" \
    TIMEOUT=${TIMEOUT:-90} \
        "$repo_root/tools/check-macaros-native-block-qemu.sh"
    mv "$check_before" "$case_output/check-before.json"

    grep -q '^result=PASS$' "$case_output/report.txt"
    grep -q "^replay_expected=$expected$" "$case_output/report.txt"
    grep -q '^checker_clean=true$' "$case_output/report.txt"

    # The report already binds these large reproducible intermediates by hash.
    # Keep compact logs and checker evidence, not six redundant package trees.
    unlink "$case_output/AFSRAM-QEMU.BND"
    unlink "$case_output/sys-composite.img"
    unlink "$case_output/sys-probe-device.fixture"
    unlink "$case_output/payload-after.img"
    rm -r "$case_output/alpha0" "$case_output/transport" "$case_output/run/esp"
done <"$fixtures/manifest.tsv"

(
    cd "$result"
    find . -type f ! -name SHA256SUMS -print | LC_ALL=C sort | \
        xargs shasum -a 256 >SHA256SUMS
)

mkdir -p "$(dirname -- "$output")"
mv "$result" "$output"
echo "macaros-native-replay-qemu result=PASS evidence=$output"
