#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause

# Hosted desktop/preferences/application session after the SYS: pivot to AFS+.
#
# The AROS tree needs more than Macaros's graft/rebuild-aros.sh builds: the
# Locale, Font, Time and other preference editors (workbench-prefs-*), the
# themes (workbench-images-themes) and codesets.library, which Locale opens
# (workbench-libs-codesets). The screenshot shows AFSPlusInfo refused by the
# Mac folder and RAM:, and answered by SYS: on AFS+.

set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
macaros_root=${MACAROS_ROOT:-"$repo_root/../Macaros"}
aros_build=${AROS_BUILD:-"$HOME/aros-build"}
aros_tree="$aros_build/bin/darwin-aarch64/AROS"
control="$macaros_root/graft/aros-ctl"
output=${AFSPLUS_HOSTED_S1B_OUTPUT:-"$repo_root/build/hosted-aros-s1b"}
work=$(mktemp -d "${TMPDIR:-/tmp}/afsplus-hosted-s1b.XXXXXX")
package="$work/package"
s1_image="$work/s1-image"
result="$work/result"
aros_started=0
installed=0

handler="$aros_tree/L/afsplus-handler"
pivot="$aros_tree/C/AFSPlusS1Pivot"
dosdriver="$aros_tree/Devs/DOSDrivers/AFSPLUS19"
image="$aros_tree/DiskImages/Unit19"

cleanup() {
    status=$?
    if [ "$aros_started" = 1 ]; then
        "$control" stop >/dev/null 2>&1 || true
    fi
    if [ "$installed" = 1 ]; then
        [ ! -e "$handler" ] || unlink "$handler"
        [ ! -e "$pivot" ] || unlink "$pivot"
        [ ! -e "$dosdriver" ] || unlink "$dosdriver"
        [ ! -e "$image" ] || unlink "$image"
    fi
    if [ "$status" -ne 0 ] && [ "${AFSPLUS_KEEP_HOSTED_FAILURE:-0}" = 1 ]; then
        echo "[hosted-s1b] keeping failed work directory: $work" >&2
        return
    fi
    [ ! -d "$work" ] || rm -r "$work"
}
trap cleanup EXIT HUP INT TERM

require_file() {
    [ -f "$1" ] || {
        echo "Missing required file: $1" >&2
        exit 66
    }
}

require_executable() {
    [ -x "$1" ] || {
        echo "Missing required executable: $1" >&2
        exit 69
    }
}

require_content() {
    actual=$(tr -d '\r\n' <"$1")
    [ "$actual" = "$2" ] || {
        echo "Unexpected content in $1: '$actual' (expected '$2')" >&2
        exit 1
    }
}

stop_aros() {
    "$control" status >"$result/status.txt" || true
    "$control" stop >/dev/null 2>&1 || true
    aros_started=0
    cp /tmp/aros-window.log "$result/aros-window.log"
    "$repo_root/tools/check-aros-serial-log.sh" "$result/aros-window.log"
}

desktop_non_background_pixels() {
    perl -e '
        use strict;
        use warnings;
        my ($file, $x1, $y1, $x2, $y2) = @ARGV;
        open my $fh, "<:raw", $file or die "$file: $!";
        my $magic = <$fh>;
        my $dims = <$fh>;
        my $maxv = <$fh>;
        die "not a P6 image" unless defined($magic) && $magic eq "P6\n";
        my ($width, $height) = split /\s+/, $dims;
        my $count = 0;
        for my $y (0 .. $height - 1) {
            read($fh, my $row, $width * 3) == $width * 3 or die "short read";
            next if $y < $y1 || $y > $y2;
            for (my $x = $x1; $x <= $x2 && $x < $width; ++$x) {
                my ($red, $green, $blue) =
                    unpack("CCC", substr($row, $x * 3, 3));
                ++$count unless $red == 153 && $green == 153 && $blue == 153;
            }
        }
        print "$count\n";
    ' "$1" 0 16 120 270
}

require_executable "$control"
require_executable "$repo_root/tools/check-aros-serial-log.sh"
require_file "$aros_tree/Devs/fdsk.device"
[ -d "$aros_tree/DiskImages" ] || {
    echo "Missing Hosted MacAROS DiskImages directory: $aros_tree" >&2
    exit 69
}
[ ! -e "$output" ] || {
    echo "Refusing to replace existing result: $output" >&2
    exit 73
}
for target in "$handler" "$pivot" "$dosdriver" "$image"; do
    [ ! -e "$target" ] || {
        echo "Refusing to replace Hosted MacAROS artifact: $target" >&2
        exit 73
    }
done
if ! "$control" status | grep -q '^state=stopped$'; then
    echo "Refusing to disturb a running Hosted MacAROS instance" >&2
    exit 75
fi

mkdir "$result"
cd "$repo_root"

echo "[hosted-s1b] build handler and target probes"
AFSPLUS_AROS_PACKAGE_OUTPUT="$package" tools/package-aros-alpha0.sh
echo "[hosted-s1b] build manifested AFS+ desktop image"
AFSPLUS_AROS_PACKAGE="$package" \
AFSPLUS_AROS_S1_OUTPUT="$s1_image" \
AFSPLUS_AROS_S1_PROFILE=desktop \
    tools/build-aros-s1-image.sh

installed=1
cp "$s1_image/afsplus-handler" "$handler"
cp "$package/AFSPlusS1Pivot" "$pivot"
cp "$s1_image/AFSPLUS19-S1" "$dosdriver"
cp "$s1_image/Unit19.s1" "$image"

echo "[hosted-s1b] boot, pivot to AFS+, then launch the desktop session"
AROS_CTL_STARTUP_MODE=minimal \
AROS_CTL_HOST_FOLDER="$result" \
AROS_CTL_STARTUP_EXTRA='Assign "FDSK:" "SYS:DiskImages"
C:Mount DEVS:DOSDrivers/AFSPLUS19 >MacRW:s1b-mount.out
C:AFSPlusS1Pivot >MacRW:s1b-pivot.out' \
    "$control" run >/dev/null
aros_started=1
"$control" wait 18 >/dev/null
"$control" tasks >"$result/s1b-tasks.out"
"$control" shot "$result/s1b-desktop.png" >/dev/null
stop_aros
printf '%s\n' none >"$result/guest-failure-requester.txt"

grep -q '^\[AFSPLUS-S1\] PASS ' "$result/s1-probe.out"
grep -q '^\[AFSPLUS-S1B\] PASS ' "$result/s1b-probe.out"
grep -q '^\[AFSPLUS-S1-PIVOT\] PASS' "$result/s1b-pivot.out"
require_content "$result/s1b-runtime" afsplus-s1b
require_content "$result/s1b-preference" afsplus-s1b
require_content "$result/s1b-env.out" afsplus-s1b
require_content "$result/s1b-saved-env" afsplus-s1b
grep -q '^{"schema":"afsplus-handler-info"' "$result/s1b-info-sys.json"
grep -q '"label":"AFSPlusS1b"' "$result/s1b-info-sys.json"
for volume in ram host; do
    grep -q 'is not served by an AFS+ handler' "$result/s1b-info-$volume.out"
done
grep -q "'WANDERER:Wanderer'" "$result/s1b-tasks.out"
grep -q 'IPrefs main' "$result/s1b-tasks.out"
grep -q 'Locale main' "$result/s1b-tasks.out"
grep -q 'Clock main' "$result/s1b-tasks.out"
require_file "$result/s1b-desktop.png"
require_file "$result/s1b-desktop.ppm"
pixels=$(desktop_non_background_pixels "$result/s1b-desktop.ppm")
[ "$pixels" -gt 800 ] || {
    echo "S1b desktop is blank or unpopulated ($pixels foreground pixels)" >&2
    exit 1
}
printf '%s\n' "$pixels" >"$result/s1b-desktop-foreground-pixels.txt"

cargo run --quiet --release -p afsplus-check --bin afsplus-check -- \
    "$image" --json >"$result/check-after.json"
grep -q '"clean":true' "$result/check-after.json"
grep -q '"log_records_pending":0' "$result/check-after.json"

cp "$image" "$result/Unit19.s1.final"
cp "$s1_image/content-SHA256SUMS" "$result/"
cp "$s1_image/check-before.json" "$result/"
cp "$package/SHA256SUMS" "$result/package-SHA256SUMS"
git -C "$macaros_root" rev-parse HEAD >"$result/macaros-commit.txt"
git -C "$macaros_root" status --short >"$result/macaros-status.txt"
(
    cd "$result"
    shasum -a 256 Unit19.s1.final check-before.json check-after.json \
        content-SHA256SUMS package-SHA256SUMS s1-probe.out s1b-probe.out \
        s1b-runtime s1b-preference s1b-env.out s1b-saved-env s1b-pivot.out \
        s1b-info-sys.json s1b-info-ram.out s1b-info-host.out \
        s1b-tasks.out \
        s1b-desktop.png s1b-desktop.ppm s1b-desktop-foreground-pixels.txt \
        macaros-commit.txt macaros-status.txt guest-failure-requester.txt \
        >SHA256SUMS
)

mkdir -p "$(dirname -- "$output")"
mv "$result" "$output"
echo "[hosted-s1b] PASS: desktop, preferences and applications run from AFS+"
echo "[hosted-s1b] result: $output"
