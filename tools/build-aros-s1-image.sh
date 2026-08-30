#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause

# Build a manifested AFS+ system image for the post-bootstrap S1 pivot.

set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
aros_build=${AROS_BUILD:-"$HOME/aros-build"}
aros_tree="$aros_build/bin/darwin-aarch64/AROS"
package=${AFSPLUS_AROS_PACKAGE:?set AFSPLUS_AROS_PACKAGE to an Alpha-0 package}
output=${AFSPLUS_AROS_S1_OUTPUT:-"$repo_root/build/aros-s1-image"}
profile=${AFSPLUS_AROS_S1_PROFILE:-core}
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

case "$profile" in
core)
    image_size_mib=64
    image_label=AFSPlusS1
    mountlist="$repo_root/native/aros/AFSPLUS19-s1.mountlist"
    sequence="$repo_root/native/aros/tests/s1-sequence"
    ;;
desktop)
    image_size_mib=256
    image_label=AFSPlusS1b
    mountlist="$repo_root/native/aros/AFSPLUS19-s1b.mountlist"
    sequence="$repo_root/native/aros/tests/s1b-sequence"
    require_file "$package/AFSPlusS1bProbe"
    require_file "$repo_root/native/aros/tests/s1b-backdrop"
    ;;
*)
    echo "Unknown S1 image profile: $profile (expected core or desktop)" >&2
    exit 64
    ;;
esac
require_file "$mountlist"
require_file "$sequence"

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
cp "$sequence" "$source_tree/S/S1-Sequence"
printf '%s' 'afsplus-s1' >"$source_tree/s1-origin"

if [ "$profile" = desktop ]; then
    desktop_commands='AddAudioModes AddDataTypes ConClip EndIf GetEnv If IPrefs LoadKeymap MakeDir Mount Run SetClock SetEnv Wait'
    desktop_directories='Classes Devs Fonts Locale Prefs System Tools Utilities L'
    for command in $desktop_commands; do
        require_file "$aros_tree/C/$command"
        cp "$aros_tree/C/$command" "$source_tree/C/$command"
    done
    for directory in $desktop_directories; do
        [ -d "$aros_tree/$directory" ] || {
            echo "Missing target desktop directory: $aros_tree/$directory" >&2
            exit 66
        }
        mkdir -p "$source_tree/$directory"
        cp -R "$aros_tree/$directory/." "$source_tree/$directory/"
        if [ -f "$aros_tree/$directory.info" ]; then
            cp "$aros_tree/$directory.info" "$source_tree/$directory.info"
        fi
    done
    mkdir -p "$source_tree/clips" "$source_tree/Prefs/Env-Archive/AFSPlus"
    cp "$package/AFSPlusS1bProbe" "$source_tree/C/AFSPlusS1bProbe"
    cp "$repo_root/native/aros/tests/s1b-backdrop" "$source_tree/.backdrop"
    printf '%s' 'afsplus-s1b' >"$source_tree/s1b-origin"
fi

(
    cd "$source_tree"
    find . -type f -print | LC_ALL=C sort | while IFS= read -r file; do
        shasum -a 256 "$file"
    done >"$result/content-SHA256SUMS"
)

cd "$repo_root"
cargo run --quiet --release -p afsplus-core --bin afsplus-mkfs -- \
    --size-mib "$image_size_mib" --label "$image_label" \
    --case-insensitive "$result/Unit19.s1"
cargo run --quiet --release -p afsplus-core --bin afsplus-populate -- \
    "$result/Unit19.s1" "$source_tree"
cargo run --quiet --release -p afsplus-check --bin afsplus-check -- \
    "$result/Unit19.s1" --json >"$result/check-before.json"
grep -q '"clean":true' "$result/check-before.json"
grep -q '"log_records_pending":0' "$result/check-before.json"

printf '%s\n' "$profile" >"$result/profile.txt"
cp "$mountlist" "$result/AFSPLUS19-S1"
cp "$package/afsplus-handler" "$result/afsplus-handler"
cp docs/aros-s1-image.md "$result/README.md"
(
    cd "$result"
    shasum -a 256 AFSPLUS19-S1 Unit19.s1 afsplus-handler profile.txt \
        check-before.json content-SHA256SUMS README.md >SHA256SUMS
)

mkdir -p "$(dirname -- "$output")"
mv "$result" "$output"
echo "[aros-s1-image] PASS: $output"
