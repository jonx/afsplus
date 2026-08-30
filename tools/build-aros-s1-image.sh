#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause

# Build a manifested AFS+ system-subset image for the post-bootstrap S1 pivot.

set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
aros_build=${AROS_BUILD:-"$HOME/aros-build"}
aros_tree="$aros_build/bin/darwin-aarch64/AROS"
package=${AFSPLUS_AROS_PACKAGE:?set AFSPLUS_AROS_PACKAGE to an Alpha-0 package}
output=${AFSPLUS_AROS_S1_OUTPUT:-"$repo_root/build/aros-s1-image"}
work=$(mktemp -d "${TMPDIR:-/tmp}/afsplus-aros-s1.XXXXXX")
source_tree="$work/source"
result="$work/result"

cleanup() {
    [ ! -d "$work" ] || rm -r "$work"
}
trap cleanup EXIT HUP INT TERM

require_file() {
    [ -f "$1" ] || {
        echo "Missing required file: $1" >&2
        exit 66
    }
}

[ ! -e "$output" ] || {
    echo "Refusing to replace existing S1 image result: $output" >&2
    exit 73
}
require_file "$package/AFSPlusS1Probe"
require_file "$package/afsplus-handler"
require_file "$repo_root/native/aros/AFSPLUS19-s1.mountlist"
require_file "$repo_root/native/aros/tests/s1-sequence"

for command in Assign Copy Execute List Path Version Which; do
    require_file "$aros_tree/C/$command"
done
[ -d "$aros_tree/Libs" ] || {
    echo "Missing target Libs directory: $aros_tree/Libs" >&2
    exit 66
}

mkdir -p "$source_tree/C" "$source_tree/L" "$source_tree/Libs" \
    "$source_tree/Devs" "$source_tree/S" "$result"
for command in Assign Copy Execute List Path Version Which; do
    cp "$aros_tree/C/$command" "$source_tree/C/$command"
done
cp "$package/AFSPlusS1Probe" "$source_tree/C/AFSPlusS1Probe"
cp -R "$aros_tree/Libs/." "$source_tree/Libs/"
cp "$repo_root/native/aros/tests/s1-sequence" "$source_tree/S/S1-Sequence"
printf '%s' 'afsplus-s1' >"$source_tree/s1-origin"

(
    cd "$source_tree"
    find . -type f -print | LC_ALL=C sort | xargs shasum -a 256 \
        >"$result/content-SHA256SUMS"
)

cd "$repo_root"
cargo run --quiet --release -p afsplus-core --bin afsplus-mkfs -- \
    --size-mib 64 --label AFSPlusS1 "$result/Unit19.s1"
cargo run --quiet --release -p afsplus-core --bin afsplus-populate -- \
    "$result/Unit19.s1" "$source_tree"
cargo run --quiet --release -p afsplus-check --bin afsplus-check -- \
    "$result/Unit19.s1" --json >"$result/check-before.json"
grep -q '"clean":true' "$result/check-before.json"
grep -q '"log_records_pending":0' "$result/check-before.json"

cp native/aros/AFSPLUS19-s1.mountlist "$result/AFSPLUS19-S1"
cp "$package/afsplus-handler" "$result/afsplus-handler"
cp docs/aros-s1-image.md "$result/README.md"
(
    cd "$result"
    shasum -a 256 AFSPLUS19-S1 Unit19.s1 afsplus-handler \
        check-before.json content-SHA256SUMS README.md >SHA256SUMS
)

mkdir -p "$(dirname -- "$output")"
mv "$result" "$output"
echo "[aros-s1-image] PASS: $output"
