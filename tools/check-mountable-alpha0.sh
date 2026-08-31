#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause

# Composite completion gate for the Mountable Alpha-0 objective. It binds the
# portable API tests, a real macFUSE/Hosted same-image round trip, and native
# MacAROS Alpha-0 plus intent-log replay into one checksummed result set.

set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
output=${AFSPLUS_MOUNTABLE_ALPHA0_OUTPUT:-"$repo_root/build/mountable-alpha0"}
reuse_result=${AFSPLUS_MOUNTABLE_ALPHA0_REUSE_RESULT:-}
# QEMU's Unix-domain QMP socket path is limited to roughly 104 bytes on macOS.
# Keep the nested native-gate result rooted in a deliberately short path.
work=$(mktemp -d /tmp/afsplus-ma0.XXXXXX)
result="$work/result"

cleanup() {
    status=$?
    if [ "$status" -ne 0 ] && [ "${AFSPLUS_KEEP_MOUNTABLE_ALPHA0_FAILURE:-0}" = 1 ]; then
        echo "[mountable-alpha0] keeping failed work directory: $work" >&2
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

require_line() {
    grep -qxF "$2" "$1" || {
        echo "Missing required verdict '$2' in $1" >&2
        exit 1
    }
}

require_clean_checker() {
    grep -q '"clean":true' "$1" || {
        echo "Checker did not report clean: $1" >&2
        exit 1
    }
    grep -q '"log_records_pending":0' "$1" || {
        echo "Checker left pending intent records: $1" >&2
        exit 1
    }
}

[ ! -e "$output" ] || {
    echo "Refusing to replace existing result: $output" >&2
    exit 73
}
for script in \
    tools/check-hosted-aros-alpha0.sh \
    tools/check-hosted-aros-crash-replay.sh \
    tools/check-macaros-native-alpha0-qemu.sh \
    tools/check-macaros-native-replay-qemu.sh
do
    require_file "$repo_root/$script"
done

cd "$repo_root"
status_before=$(git status --porcelain=v1)

if [ -n "$reuse_result" ]; then
    [ -d "$reuse_result" ] || {
        echo "Missing reusable composite result: $reuse_result" >&2
        exit 66
    }
    reuse_source=$(CDPATH= cd -- "$reuse_result" && pwd)
    mkdir "$result"
    cp -R "$reuse_source"/. "$result"/
    echo "[mountable-alpha0] reuse completed sub-gates from $reuse_source"
else
    mkdir "$result"

    echo "[mountable-alpha0] portable VFS contract"
    cargo test -p afsplus-vfs --test api >"$result/vfs-tests.log" 2>&1

    echo "[mountable-alpha0] host FUSE protocol contract"
    cargo test -p afsplus-fuse --test protocol >"$result/fuse-tests.log" 2>&1

    echo "[mountable-alpha0] intent-log crash contract"
    cargo test -p afsplus-check --test intent_log >"$result/intent-log-tests.log" 2>&1

    echo "[mountable-alpha0] AROS DOS adapter contract"
    cargo test -p afsplus-aros --test adapter >"$result/aros-adapter-tests.log" 2>&1

    echo "[mountable-alpha0] Hosted AROS -> macFUSE -> Hosted AROS"
    AFSPLUS_HOSTED_RESULT_OUTPUT="$result/hosted-alpha0" \
        tools/check-hosted-aros-alpha0.sh

    echo "[mountable-alpha0] Hosted intent-log replay"
    AFSPLUS_HOSTED_REPLAY_OUTPUT="$result/hosted-replay" \
        tools/check-hosted-aros-crash-replay.sh

    echo "[mountable-alpha0] native MacAROS Alpha-0"
    AFSPLUS_MACAROS_QEMU_OUTPUT="$result/native-alpha0" \
        tools/check-macaros-native-alpha0-qemu.sh

    echo "[mountable-alpha0] native MacAROS intent-log replay"
    AFSPLUS_MACAROS_REPLAY_OUTPUT="$result/native-replay" \
        tools/check-macaros-native-replay-qemu.sh
fi

echo "[mountable-alpha0] verify Hosted same-image evidence"
for checker in check-after-target.json check-after-host.json check-final.json
do
    require_clean_checker "$result/hosted-alpha0/$checker"
done
require_line "$result/hosted-alpha0/guest-failure-requester.txt" none
require_line "$result/hosted-alpha0/target/probe.out" \
    '[AFSPLUS-ALPHA0] PASS create/read/write/truncate/rename/fsync/casefold'
grep -q '^\[AFSPLUS-HOST-ALPHA0\] PASS ' \
    "$result/hosted-alpha0/host-probe.out"
[ "$(cat "$result/hosted-alpha0/return/alpha0.from-host")" = host ]
[ "$(cat "$result/hosted-alpha0/return/alpha0.from-aros")" = hello ]
for status in shutdown-1 restart-1 shutdown-2 restart-2 shutdown-final \
    dismount-final
do
    require_line "$result/hosted-alpha0/return/$status.status" pass
done
(
    cd "$result/hosted-alpha0"
    shasum -a 256 -c SHA256SUMS >/dev/null
)

echo "[mountable-alpha0] verify Hosted replay evidence"
hosted_replay_cases=0
for checker in "$result"/hosted-replay/cases/*/check-after.json
do
    require_clean_checker "$checker"
    hosted_replay_cases=$((hosted_replay_cases + 1))
done
[ "$hosted_replay_cases" -eq 6 ]
require_line "$result/hosted-replay/guest-failure-requester.txt" none
(
    cd "$result/hosted-replay"
    shasum -a 256 -c SHA256SUMS >/dev/null
)

echo "[mountable-alpha0] verify native MacAROS evidence"
require_line "$result/native-alpha0/report.txt" result=PASS
require_line "$result/native-alpha0/report.txt" mode=alpha0
require_line "$result/native-alpha0/report.txt" checker_clean=true
require_line "$result/native-alpha0/report.txt" guest_failure_requester=none
require_clean_checker "$result/native-alpha0/check-after.json"

native_replay_cases=0
native_old_cases=0
native_new_cases=0
for report in "$result"/native-replay/cases/*/report.txt
do
    require_line "$report" result=PASS
    require_line "$report" checker_clean=true
    require_line "$report" guest_failure_requester=none
    case $(sed -n 's/^replay_expected=//p' "$report") in
    old) native_old_cases=$((native_old_cases + 1)) ;;
    new) native_new_cases=$((native_new_cases + 1)) ;;
    *) echo "Missing native replay outcome in $report" >&2; exit 1 ;;
    esac
    native_replay_cases=$((native_replay_cases + 1))
done
[ "$native_replay_cases" -eq 6 ]
[ "$native_old_cases" -eq 4 ]
[ "$native_new_cases" -eq 2 ]
(
    cd "$result/native-replay"
    shasum -a 256 -c SHA256SUMS >/dev/null
)

# Match the compact native replay evidence policy: the report binds these
# reproducible large intermediates before they are removed from the final set.
for compact_file in AFSRAM-QEMU.BND sys-composite.img \
    sys-probe-device.fixture payload-after.img
do
    [ ! -e "$result/native-alpha0/$compact_file" ] || \
        unlink "$result/native-alpha0/$compact_file"
done
for compact_dir in alpha0 transport run/esp
do
    [ ! -d "$result/native-alpha0/$compact_dir" ] || \
        rm -r "$result/native-alpha0/$compact_dir"
done

status_after=$(git status --porcelain=v1)
[ "$status_before" = "$status_after" ] || {
    echo "AFS+ worktree changed during the composite gate" >&2
    exit 1
}

{
    printf 'requirement\tstatus\tevidence\n'
    printf 'mount_and_intent_log_hardening\tPASS\tintent-log tests + Hosted/native replay\n'
    printf 'portable_vfs_api\tPASS\tafsplus-vfs API tests\n'
    printf 'host_fuse_mount\tPASS\treal macFUSE same-image phase\n'
    printf 'macaros_integration\tPASS\tHosted AROS and native Apple-AArch64 QEMU\n'
    printf 'same_image_operations\tPASS\tcreate/read/write/truncate/rename/fsync cross-readback\n'
    printf 'crash_replay\tPASS\t6 Hosted + 6 native old/new cases\n'
    printf 'strict_clean_verification\tPASS\tchecker clean and zero pending records at every boundary\n'
    printf 'guest_failure_detection\tPASS\trequester/fatal scan on every runtime channel\n'
} >"$result/requirements.tsv"

{
    echo "format=afsplus-mountable-alpha0-v1"
    echo "result=PASS"
    echo "hosted_replay_cases=$hosted_replay_cases"
    echo "native_replay_cases=$native_replay_cases"
    echo "native_replay_old=$native_old_cases"
    echo "native_replay_new=$native_new_cases"
    echo "guest_failure_requester=none"
    echo "hardware_claim=none"
} >"$result/report.txt"

(
    cd "$result"
    find . -type f ! -path './SHA256SUMS' -print | LC_ALL=C sort | \
        xargs shasum -a 256 >SHA256SUMS
)

mkdir -p "$(dirname -- "$output")"
mv "$result" "$output"
echo "mountable-alpha0 result=PASS evidence=$output"
