#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause

# S3 on Hosted MacAROS: AROS booted from an AFS+ partition survives being cut
# off, round after round. The partition, the disk image and the boot modules
# are S2's (tools/check-hosted-aros-s2.sh); what is new is that the machine is
# killed in the middle of its work and started again, N times.
#
# Each round boots from the AFS+ partition, lets the Startup-Sequence do real
# work -- its own commands, Wanderer, and AFSPlusS3Workload, which churns
# files, saves a preference under ENVARC: and writes two markers -- and then
# kills the AROS process at a moment the schedule chose: during the
# Startup-Sequence before the desktop, during the workload's writes, just
# after a marker was made durable with ACTION_FLUSH, during the idle time in
# which a delayed-commit mount cleans its deletes, or not at all, the clean
# shutdown a round in five uses as its reference. The schedule comes from the
# round number and one seed and is printed before anything boots.
#
# After every cut the partition is taken out and checked, the machine is
# booted again and must reach the Startup-Sequence on AFS+, and that boot
# copies every marker out for the host to judge (tools/s3-markers.py): a
# marker claimed flushed before the cut must be there and byte for byte the
# same, one only written may be there or not but never half written. The
# partition must check clean after the cut, and hold no pending log records
# after the mount that follows it. Boot time is recorded per round: the S3
# leak clause fails a run whose last three boots each take more than twice
# the mean of the first three.
#
#   AFSPLUS_S3_ROUNDS   rounds (default 24; 6 is a valid development run)
#   AFSPLUS_S3_SEED     the schedule seed (default 20260919)
#   AFSPLUS_S3_MOMENT   pin every round to one moment, for a control run
#   AFSPLUS_S3_CONTROL  1 makes the workload skip the flush and claim it
#                       anyway: the negative control, which must FAIL
#   AFSPLUS_S3_OUTPUT   where the result goes (default build/hosted-aros-s3)
#   AFSPLUS_S3_TIMEOUT  seconds a boot may take (default 180)

set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
macaros_root=${MACAROS_ROOT:-"$repo_root/../Macaros"}
aros_build=${AROS_BUILD:-"$HOME/aros-build"}
aros_tree="$aros_build/bin/darwin-aarch64/AROS"
boot_conf="$aros_tree/boot/darwin/AROSBootstrap.conf"
control="$macaros_root/graft/aros-ctl"
timeout=${AFSPLUS_S3_TIMEOUT:-180}
rounds=${AFSPLUS_S3_ROUNDS:-24}
seed=${AFSPLUS_S3_SEED:-20260919}
moment_pin=${AFSPLUS_S3_MOMENT:-}
skip_flush=${AFSPLUS_S3_CONTROL:-0}
output=${AFSPLUS_S3_OUTPUT:-"$repo_root/build/hosted-aros-s3"}
work=$(mktemp -d "${TMPDIR:-/tmp}/afsplus-s3.XXXXXX")
package="$work/package"
source_tree="$work/system"
result="$work/result"
aros_started=0
installed=0

handler="$aros_tree/L/afsplus-handler"
hostdisk="$aros_tree/Devs/hostdisk.device"
handler_line="module $handler"
hostdisk_line="module $hostdisk"
partition="$aros_tree/Libs/partition.library"
partition_line="module $partition"

remove_module_lines() {
    [ -f "$boot_conf" ] || return 0
    grep -vxF -e "$handler_line" -e "$hostdisk_line" -e "$partition_line" \
        "$boot_conf" | grep -v '^arguments .*hostdisk=' \
        | grep -v '^memory ' >"$work/conf" || true
    cat "$work/conf" >"$boot_conf"
}

cleanup() {
    status=$?
    if [ "$aros_started" = 1 ]; then
        "$control" kill >/dev/null 2>&1 || true
    fi
    remove_module_lines
    if [ "$installed" = 1 ] && [ -e "$handler" ]; then
        unlink "$handler"
    fi
    if [ "$status" -ne 0 ] && [ "${AFSPLUS_KEEP_HOSTED_FAILURE:-0}" = 1 ]; then
        echo "[hosted-s3] keeping failed work directory: $work" >&2
        return
    fi
    [ ! -d "$work" ] || rm -r "$work"
}
trap cleanup EXIT HUP INT TERM

[ -x "$control" ] || { echo "Missing $control" >&2; exit 69; }
[ -f "$boot_conf" ] || { echo "Missing $boot_conf" >&2; exit 69; }
[ -f "$hostdisk" ] || {
    echo "Missing $hostdisk; build workbench-devs-hostdisk with native/aros/aros-patches" >&2
    exit 69
}
for path in Libs/partition.library Prefs/Presets/Themes Prefs/Locale Libs/codesets.library System/Wanderer/Wanderer; do
    [ -e "$aros_tree/$path" ] || {
        echo "Missing $aros_tree/$path; see tools/check-hosted-aros-s1b.sh" >&2
        exit 69
    }
done
case "$rounds" in
    ''|*[!0-9]*) echo "AFSPLUS_S3_ROUNDS must be a number" >&2; exit 64;;
esac
[ "$rounds" -ge 1 ] || { echo "AFSPLUS_S3_ROUNDS must be at least 1" >&2; exit 64; }
[ ! -e "$output" ] || {
    echo "Refusing to replace existing result: $output" >&2
    exit 73
}
[ ! -e "$handler" ] || {
    echo "Refusing to replace Hosted MacAROS artifact: $handler" >&2
    exit 73
}
if grep -qxF -e "$handler_line" -e "$hostdisk_line" -e "$partition_line" "$boot_conf"; then
    echo "The boot module list already names the handler, hostdisk or partition.library" >&2
    exit 73
fi
if ! "$control" status | grep -q '^state=stopped$'; then
    echo "Refusing to disturb a running Hosted MacAROS instance" >&2
    exit 75
fi

mkdir "$result"
cd "$repo_root"

echo "[hosted-s3] the host side decides first"
python3 tools/test-s3-markers.py >"$result/selftest.txt" 2>&1 || {
    echo "[hosted-s3] the host-side self-test fails; nothing is booted" >&2
    cat "$result/selftest.txt" >&2
    exit 1
}
tail -n 3 "$result/selftest.txt"
if [ -n "$moment_pin" ]; then
    python3 tools/s3-markers.py schedule --seed "$seed" --rounds "$rounds" \
        --moment "$moment_pin" >"$result/schedule.txt"
else
    python3 tools/s3-markers.py schedule --seed "$seed" --rounds "$rounds" \
        >"$result/schedule.txt"
fi
echo "[hosted-s3] schedule, seed $seed, $rounds rounds:"
sed 's/^/[hosted-s3]   round /' "$result/schedule.txt"
if [ "$skip_flush" = 1 ]; then
    echo "[hosted-s3] NEGATIVE CONTROL: the workload skips the flush and"
    echo "[hosted-s3] claims the marker as flushed anyway; this run must FAIL"
fi

echo "[hosted-s3] build a fresh qualified package"
AFSPLUS_AROS_PACKAGE_OUTPUT="$package" tools/package-aros-alpha0.sh
(cd "$package" && shasum -a 256 -c SHA256SUMS >/dev/null)

echo "[hosted-s3] copy the system onto an AFS+ partition"
mkdir "$source_tree"
for entry in "$aros_tree"/*; do
    case "${entry##*/}" in
        Developer|DiskImages|boot|S) ;;
        *) cp -R "$entry" "$source_tree/" ;;
    esac
done
# AROS.boot comes along: dos.library boots only from a volume whose AROS.boot
# names its CPU (rom/dos/isbootable.c).
[ -f "$source_tree/AROS.boot" ]
mkdir "$source_tree/S"
cp native/aros/tests/s3-startup-sequence "$source_tree/S/Startup-Sequence"
cp "$package/AFSPlusS3Workload" "$source_tree/C/AFSPlusS3Workload"
cp "$package/AFSPlusInfo" "$source_tree/C/AFSPlusInfo"
cp "$package/afsplus-handler" "$source_tree/L/afsplus-handler"
cargo run --quiet --release -p afsplus-tools --bin mkafsplus -- \
    --profile workstation --size-mib 128 --label AFSPlusS3 \
    --case-insensitive "$work/system.afsp" >/dev/null
cargo run --quiet --release -p afsplus-core --bin afsplus-populate -- \
    "$work/system.afsp" "$source_tree" >"$result/populate.txt"
cargo run --quiet --release -p afsplus-check --bin afsplus-check -- \
    "$work/system.afsp" --json >"$result/check-before.json"
grep -q '"clean":true' "$result/check-before.json"
cargo run --quiet --release -p afsplus-tools --bin afsplus-disk -- \
    wrap --bootpri 10 --name AFSSYS "$work/disk0.img" "$work/system.afsp" \
    >"$result/wrap.txt"
rm "$work/system.afsp"

installed=1
cp "$package/afsplus-handler" "$handler"
printf '%s\n%s\n%s\n' "$partition_line" "$hostdisk_line" "$handler_line" \
    >>"$boot_conf"

now_seconds() {
    python3 -c 'import time; print("%.1f" % time.time())'
}

sleep_ms() {
    sleep "$(awk "BEGIN { printf \"%.3f\", $1 / 1000 }")"
}

wait_stopped() {
    left=100
    while [ "$left" -gt 0 ]; do
        if "$control" status 2>/dev/null | grep -q '^state=stopped$'; then
            return 0
        fi
        sleep 0.2
        left=$((left - 1))
    done
    return 1
}

start_boot() {
    run_dir=$1
    wait_stopped || {
        echo "[hosted-s3] a Hosted MacAROS instance would not stop" >&2
        exit 75
    }
    AROS_HOST_ARGS="hostdisk=$work/disk%ld.img bootdevice=AFSSYS" \
    AROS_CTL_STARTUP_MODE=minimal \
    AROS_CTL_HOST_FOLDER="$run_dir" \
    AROS_CTL_STARTUP_EXTRA='C:Echo host >MacRW:done' \
        "$control" run >"$run_dir/run.out"
    aros_started=1
}

# Waits for $2 to appear in the file $1, within $3 seconds. An empty pattern
# waits for the file to hold anything at all: a guest Echo creates the file
# before it writes it, and a poll that only asks whether it exists reads it
# empty. Returns 1 on timeout, so a fixed sleep can take over and the round
# says so.
wait_for() {
    file=$1
    pattern=$2
    left=$(($3 * 5))
    while [ "$left" -gt 0 ]; do
        if [ -z "$pattern" ]; then
            if [ -s "$file" ]; then
                sleep 0.3
                return 0
            fi
        elif [ -f "$file" ] && grep -q "$pattern" "$file" 2>/dev/null; then
            return 0
        fi
        if "$control" status 2>/dev/null | grep -q '^state=stopped$'; then
            # The instance is gone: nothing more will be written.
            return 1
        fi
        sleep 0.2
        left=$((left - 1))
    done
    return 1
}

stage_now() {
    if [ -f "$1/s3-stage" ]; then
        tail -n 1 "$1/s3-stage" | awk '{print $2}'
    else
        echo none
    fi
}

extract_and_check() {
    cargo run --quiet --release -p afsplus-tools --bin afsplus-disk -- \
        extract "$work/disk0.img" "$work/after.afsp" >/dev/null
    cargo run --quiet --release -p afsplus-check --bin afsplus-check -- \
        "$work/after.afsp" --json >"$1"
    rm -f "$work/after.afsp"
}

round=1
while [ "$round" -le "$rounds" ]; do
    moment=$(awk -v r="$round" '$1 == r { print $2 }' "$result/schedule.txt")
    jitter=$(awk -v r="$round" '$1 == r { print $3 }' "$result/schedule.txt")
    round_dir="$result/round$round"
    run_dir="$round_dir/work"
    verify_dir="$round_dir/verify"
    mkdir -p "$run_dir" "$verify_dir"
    if [ "$skip_flush" = 1 ]; then
        printf 'work %s %s noflush\n' "$round" "$seed" >"$run_dir/s3-control"
    else
        printf 'work %s %s\n' "$round" "$seed" >"$run_dir/s3-control"
    fi
    printf 'verify %s %s\n' "$round" "$seed" >"$verify_dir/s3-control"

    echo "[hosted-s3] round $round/$rounds: cut at $moment, ${jitter} ms in"
    start_boot "$run_dir"
    timed_out=no
    case "$moment" in
        startup)
            wait_for "$run_dir/s3-stage" 'stage startup' "$timeout" || timed_out=yes
            sleep_ms "$jitter"
            ;;
        writes)
            wait_for "$run_dir/s3-progress" 'phase churn' "$timeout" || timed_out=yes
            sleep_ms "$jitter"
            ;;
        after-flush)
            wait_for "$run_dir/s3-progress" "marker $round flushed" "$timeout" \
                || timed_out=yes
            sleep_ms "$jitter"
            ;;
        idle)
            wait_for "$run_dir/s3-progress" 'phase idle' "$timeout" || timed_out=yes
            sleep_ms "$jitter"
            ;;
        clean)
            wait_for "$run_dir/done" '' "$timeout" || timed_out=yes
            ;;
        *)
            echo "[hosted-s3] round $round has no moment" >&2
            exit 70
            ;;
    esac
    if [ "$timed_out" = yes ]; then
        echo "[hosted-s3] round $round: the boot never reached $moment;" \
            "cutting after the fixed wait instead" >&2
        "$control" shot "$round_dir/timeout.png" >/dev/null 2>&1 || true
    fi
    stage=$(stage_now "$run_dir")
    claimed=""
    if grep -q "marker $round flushed" "$run_dir/s3-progress" 2>/dev/null; then
        claimed="$claimed --claimed-flush"
    fi
    if grep -q "marker $round written" "$run_dir/s3-progress" 2>/dev/null; then
        claimed="$claimed --claimed-soft"
    fi
    if [ "$moment" = clean ]; then
        "$control" stop >"$round_dir/stop.out" 2>&1 || true
    else
        "$control" kill >"$round_dir/kill.out" 2>&1 || true
    fi
    aros_started=0
    tail -n 200 /tmp/aros-window.log >"$round_dir/work-window.log" 2>/dev/null || true

    extract_and_check "$round_dir/check-cut.json"
    clean=yes
    grep -q '"clean":true' "$round_dir/check-cut.json" || clean=no

    start_boot "$verify_dir"
    started=$(now_seconds)
    boot_ok=no
    if wait_for "$verify_dir/done" '' "$timeout"; then
        if [ "$(cat "$verify_dir/done")" = afsplus ]; then
            boot_ok=yes
        fi
    fi
    boot_seconds=$(awk "BEGIN { printf \"%.1f\", $(now_seconds) - $started }")
    if [ "$round" = 1 ]; then
        "$control" shot "$result/desktop.png" >/dev/null 2>&1 || true
    fi
    # The verify boot always ends cleanly: what follows it must hold no
    # pending log records, and that is a statement about the mount, not about
    # a second cut.
    "$control" stop >"$round_dir/verify-stop.out" 2>&1 || true
    aros_started=0
    tail -n 200 /tmp/aros-window.log >"$round_dir/verify-window.log" 2>/dev/null || true

    extract_and_check "$round_dir/check-verify.json"
    grep -q '"clean":true' "$round_dir/check-verify.json" || clean=no
    pending=$(sed -n 's/.*"log_records_pending":\([0-9]*\).*/\1/p' \
        "$round_dir/check-verify.json" | head -n 1)
    [ -n "$pending" ] || pending=-1

    # shellcheck disable=SC2086 -- $claimed is a built list of flags.
    python3 tools/s3-markers.py record --state "$result/state.json" \
        --markers "$verify_dir/markers" --seed "$seed" --round "$round" \
        --moment "$moment" --jitter-ms "$jitter" --stage "$stage" \
        --boot-ok "$boot_ok" --clean "$clean" --pending "$pending" \
        --boot-seconds "$boot_seconds" $claimed
    echo "[hosted-s3] round $round: stage $stage, boot $boot_ok," \
        "clean $clean, pending $pending, ${boot_seconds}s"
    round=$((round + 1))
done

remove_module_lines
unlink "$handler"
installed=0

echo "[hosted-s3] the rounds, and the verdict"
status=0
python3 tools/s3-markers.py summary --state "$result/state.json" \
    --output "$result/rounds.txt" || status=$?
printf '%s\n' none >"$result/guest-failure-requester.txt"
cp "$package/SHA256SUMS" "$result/package-SHA256SUMS"
(
    cd "$result"
    shasum -a 256 schedule.txt rounds.txt state.json check-before.json \
        selftest.txt guest-failure-requester.txt package-SHA256SUMS \
        >SHA256SUMS
)
mkdir -p "$(dirname -- "$output")"
mv "$result" "$output"
if [ "$status" -ne 0 ]; then
    echo "[hosted-s3] FAIL: see $output/rounds.txt"
    echo "[hosted-s3] result: $output"
    exit 1
fi
echo "[hosted-s3] PASS: AROS booted from AFS+ survived $rounds cuts"
echo "[hosted-s3] result: $output"
