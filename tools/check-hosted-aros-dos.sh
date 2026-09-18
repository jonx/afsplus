#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause

# DOS semantics beyond the Alpha-0 slice through a real dos.library on Hosted
# MacAROS, then a dismount after a notification message that is never replied.

set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
macaros_root=${MACAROS_ROOT:-"$repo_root/../Macaros"}
aros_build=${AROS_BUILD:-"$HOME/aros-build"}
aros_tree="$aros_build/bin/darwin-aarch64/AROS"
control="$macaros_root/graft/aros-ctl"
output=${AFSPLUS_HOSTED_DOS_OUTPUT:-"$repo_root/build/hosted-aros-dos"}
work=$(mktemp -d "${TMPDIR:-/tmp}/afsplus-hosted-dos.XXXXXX")
package="$work/package"
result="$work/result"
aros_started=0
installed=0

handler="$aros_tree/L/afsplus-handler"
probe="$aros_tree/C/AFSPlusDosProbe"
info="$aros_tree/C/AFSPlusInfo"
clone="$aros_tree/C/AFSPlusClone"
dosdriver="$aros_tree/Devs/DOSDrivers/AFSPLUS19"
image="$aros_tree/DiskImages/Unit19"

cleanup() {
    status=$?
    if [ "$aros_started" = 1 ]; then
        "$control" stop >/dev/null 2>&1 || true
    fi
    if [ "$installed" = 1 ]; then
        [ ! -e "$handler" ] || unlink "$handler"
        [ ! -e "$probe" ] || unlink "$probe"
        [ ! -e "$info" ] || unlink "$info"
        [ ! -e "$clone" ] || unlink "$clone"
        [ ! -e "$dosdriver" ] || unlink "$dosdriver"
        [ ! -e "$image" ] || unlink "$image"
    fi
    if [ "$status" -ne 0 ] && [ "${AFSPLUS_KEEP_HOSTED_FAILURE:-0}" = 1 ]; then
        echo "[hosted-dos] keeping failed work directory: $work" >&2
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

start_aros() {
    phase_result=$1
    startup=$2
    AROS_CTL_STARTUP_MODE=minimal \
    AROS_CTL_HOST_FOLDER="$phase_result" \
    AROS_CTL_STARTUP_EXTRA="$startup" \
        "$control" run >/dev/null
    aros_started=1
    "$control" wait 8 >/dev/null
}

stop_aros() {
    phase_result=$1
    "$control" status >"$phase_result/status.txt" || true
    "$control" stop >/dev/null 2>&1 || true
    aros_started=0
    cp /tmp/aros-window.log "$phase_result/aros-window.log"
    "$repo_root/tools/check-aros-serial-log.sh" \
        "$phase_result/aros-window.log"
}

check_image() {
    cargo run --quiet --release -p afsplus-check --bin afsplus-check -- \
        "$image" --json >"$1"
    grep -q '"clean":true' "$1"
}

# A probe prints its verdict line and may still print a FAIL after it, as
# STEADY does when its bound is crossed; a FAIL anywhere fails the gate.
no_probe_failure() {
    if grep -q '^\[AFSPLUS-DOS\] FAIL ' "$1"; then
        grep '^\[AFSPLUS-DOS\] FAIL ' "$1" >&2
        exit 1
    fi
}

require_executable "$control"
require_executable "$repo_root/tools/check-aros-serial-log.sh"
require_file "$aros_tree/Devs/fdsk.device"
require_file "$aros_tree/C/Mount"
require_file "$aros_tree/C/Assign"
[ -d "$aros_tree/DiskImages" ] || {
    echo "Missing Hosted MacAROS DiskImages directory: $aros_tree" >&2
    exit 69
}
[ ! -e "$output" ] || {
    echo "Refusing to replace existing result: $output" >&2
    exit 73
}
for target in "$handler" "$probe" "$info" "$clone" "$dosdriver" "$image"; do
    [ ! -e "$target" ] || {
        echo "Refusing to replace Hosted MacAROS artifact: $target" >&2
        exit 73
    }
done
if ! "$control" status | grep -q '^state=stopped$'; then
    echo "Refusing to disturb a running Hosted MacAROS instance" >&2
    exit 75
fi

mkdir "$result" "$result/dos" "$result/records" "$result/hold"
cd "$repo_root"

echo "[hosted-dos] build a fresh qualified package"
AFSPLUS_AROS_PACKAGE_OUTPUT="$package" tools/package-aros-alpha0.sh

installed=1
cp "$package/afsplus-handler" "$handler"
cp "$package/AFSPlusDosProbe" "$probe"
cp "$package/AFSPlusInfo" "$info"
cp "$package/AFSPlusClone" "$clone"
cp "$package/AFSPLUS19" "$dosdriver"
cp "$package/Unit19" "$image"

echo "[hosted-dos] DOS semantics phase"
start_aros "$result/dos" \
    'C:FailAt 21
Assign "FDSK:" "SYS:DiskImages"
C:Mount DEVS:DOSDrivers/AFSPLUS19 >MacRW:mount.out
C:AFSPlusDosProbe >MacRW:probe.out
C:List AFSPLUS19: ALL >MacRW:list-after.out
C:AFSPlusInfo AFSPLUS19: >MacRW:info.json
C:AFSPlusInfo SYS: >MacRW:info-foreign.out
C:Copy C:Mount AFSPLUS19:clone.src >MacRW:clone-source.out
C:AFSPlusClone AFSPLUS19:clone.src AFSPLUS19: clone.dst >MacRW:clone.out
C:AFSPlusClone AFSPLUS19:clone.src AFSPLUS19: clone.dst >MacRW:clone-again.out
C:AFSPlusClone AFSPLUS19:clone.src RAM: clone.dst >MacRW:clone-ram.out
C:Copy AFSPLUS19:clone.dst MacRW:clone.dst >MacRW:clone-copy.out
C:Copy RAM:clone.dst MacRW:clone-ram.dst >MacRW:clone-ram-copy.out
C:AFSPlusDosProbe STEADY 100 >MacRW:steady.out
C:AFSPlusInfo AFSPLUS19: PACKETS >MacRW:packets.out
C:Mount AFSPLUS19: SHUTDOWN >MacRW:shutdown.out
If WARN
    C:Echo fail >MacRW:shutdown.status
Else
    C:Echo pass >MacRW:shutdown.status
EndIf'
stop_aros "$result/dos"
no_probe_failure "$result/dos/probe.out"
grep -q '^\[AFSPLUS-DOS\] PASS ' "$result/dos/probe.out"
grep -q '^\[AFSPLUS-DOS\] v2 watch taken 1 then 0, removed$' "$result/dos/probe.out"
grep '^\[AFSPLUS-DOS\] self-overlap ' "$result/dos/probe.out" \
    >"$result/record-self-overlap.txt"
[ "$(cat "$result/dos/shutdown.status")" = pass ]
if grep -q 'dosprobe' "$result/dos/list-after.out"; then
    echo "The probe left its drawer behind" >&2
    exit 1
fi
# The report reached an application through the extension packet, and a
# handler that does not know the packet was recognised as such.
python3 -c 'import json, sys
document = json.load(open(sys.argv[1]))
assert document["schema"] == "afsplus-handler-info", document
assert document["schema_version"] == 1, document' "$result/dos/info.json"
cp "$result/dos/info.json" "$result/handler-info.json"
grep -q 'not served by an AFS+ handler' "$result/dos/info-foreign.out"
# One tool, three answers: a clone inside the volume, a refusal to replace,
# and a byte copy to a handler without the transport. Both results carry the
# source's bytes; the checker below judges the shared extents.
grep -q '^AFSPlusClone: cloned$' "$result/dos/clone.out"
grep -q 'error 203$' "$result/dos/clone-again.out"
grep -q '^AFSPlusClone: copied$' "$result/dos/clone-ram.out"
cmp "$aros_tree/C/Mount" "$result/dos/clone.dst"
cmp "$aros_tree/C/Mount" "$result/dos/clone-ram.dst"
rm "$result/dos/clone.dst" "$result/dos/clone-ram.dst"
# What dos.library really sent, by packet type and by error code, kept as an
# observation next to the probe's own view. One fact is required of it: the
# comment travelled as ACTION_SET_COMMENT (28), not through an emulation.
# A hundred rounds of paired operations must cost the system nothing.
no_probe_failure "$result/dos/steady.out"
grep -q '^\[AFSPLUS-DOS\] STEADY rounds 100 ' "$result/dos/steady.out"
grep -q '^\[AFSPLUS-DOS\] STEADY heap before [1-9][0-9]* ' "$result/dos/steady.out"
cp "$result/dos/steady.out" "$result/steady.txt"
cp "$result/dos/packets.out" "$result/packets.txt"
grep -Eq '^packet 28 [1-9][0-9]* ' "$result/packets.txt"
check_image "$result/check-after-dos.json"
# The attribute the probe left on the volume root, read from the image by the
# host: what travelled through the extension packet is what is stored.
cargo run --quiet --release -p afsplus-tools --bin afsplus-explain -- \
    --json "$image" path / >"$result/root-explain.json"
python3 -c 'import json, sys
document = json.load(open(sys.argv[1]))
attributes = json.dumps(document["object"]["attributes"])
assert "aros.probe" in attributes, attributes' "$result/root-explain.json"

echo "[hosted-dos] a waiting record lock granted by another task's release"
start_aros "$result/records" \
    'C:FailAt 21
Assign "FDSK:" "SYS:DiskImages"
C:Mount DEVS:DOSDrivers/AFSPLUS19 >MacRW:mount.out
C:Run >NIL: C:AFSPlusDosProbe RECORD-HOLDER
C:AFSPlusDosProbe RECORD-WAITER >MacRW:records.out
C:Mount AFSPLUS19: SHUTDOWN >MacRW:shutdown.out
If WARN
    C:Echo fail >MacRW:shutdown.status
Else
    C:Echo pass >MacRW:shutdown.status
EndIf'
stop_aros "$result/records"
no_probe_failure "$result/records/records.out"
grep -q '^\[AFSPLUS-DOS\] RECORDS granted after ' "$result/records/records.out"
[ "$(cat "$result/records/shutdown.status")" = pass ]
check_image "$result/check-after-records.json"

echo "[hosted-dos] dismount with an unreplied notification"
start_aros "$result/hold" \
    'C:FailAt 21
Assign "FDSK:" "SYS:DiskImages"
C:Mount DEVS:DOSDrivers/AFSPLUS19 >MacRW:mount.out
C:AFSPlusDosProbe HOLD >MacRW:hold.out
C:Mount AFSPLUS19: SHUTDOWN >MacRW:shutdown.out
If WARN
    C:Echo fail >MacRW:shutdown.status
Else
    C:Echo pass >MacRW:shutdown.status
EndIf
C:Assign AFSPLUS19: DISMOUNT >MacRW:dismount.out
If WARN
    C:Echo fail >MacRW:dismount.status
Else
    C:Echo pass >MacRW:dismount.status
EndIf
C:Echo alive >MacRW:after-dismount.status'
stop_aros "$result/hold"
no_probe_failure "$result/hold/hold.out"
grep -q '^\[AFSPLUS-DOS\] HOLD ' "$result/hold/hold.out"
[ "$(cat "$result/hold/shutdown.status")" = pass ]
[ "$(cat "$result/hold/dismount.status")" = pass ]
[ "$(cat "$result/hold/after-dismount.status")" = alive ]
check_image "$result/check-after-hold.json"

printf '%s\n' none >"$result/guest-failure-requester.txt"
cp "$package/SHA256SUMS" "$result/package-SHA256SUMS"
(
    cd "$result"
    shasum -a 256 check-after-dos.json check-after-records.json \
        check-after-hold.json \
        record-self-overlap.txt packets.txt steady.txt handler-info.json \
        guest-failure-requester.txt >SHA256SUMS
)

mkdir -p "$(dirname -- "$output")"
mv "$result" "$output"
echo "[hosted-dos] PASS: $output"
