#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause

# Fail when an AROS diagnostic stream contains a modal software-failure
# requester or another fatal runtime marker used by the Hosted gates. Such a
# failure can leave the emulator alive until its timeout, so process status
# alone is not a sufficient guest verdict.

set -eu

check_log() {
    log=$1
    [ -f "$log" ] || {
        echo "Missing AROS diagnostic log: $log" >&2
        return 66
    }

    failure_pattern='Software Failure!|Guru Meditation'
    failure_pattern="$failure_pattern|AFSPLUS.*failed|Trap signal|ALERT"
    failure_pattern="$failure_pattern|unrecoverable|halting host"
    if LC_ALL=C grep -Eq "$failure_pattern" "$log"; then
        echo "Fatal AROS diagnostic detected in $log:" >&2
        diagnostic_pattern="$failure_pattern|Task[[:space:]]*:|Error:"
        diagnostic_pattern="$diagnostic_pattern|PC[[:space:]]*:|Module |Function "
        LC_ALL=C grep -En \
            "$diagnostic_pattern" \
            "$log" | tail -30 >&2 || true
        return 1
    fi
}

if [ "$#" -eq 1 ] && [ "$1" = --self-test ]; then
    test_work=$(mktemp -d "${TMPDIR:-/tmp}/afsplus-aros-serial.XXXXXX")
    cleanup() {
        rm -r "$test_work"
    }
    trap cleanup EXIT HUP INT TERM
    printf '%s\n' '[AFSPLUS-ALPHA0] PASS' >"$test_work/clean.log"
    printf '%s\n' \
        'Software Failure!' \
        'Task : 0x002E0A18 - AFSPLUS19' \
        'Error: 0x80000004 - Illegal instruction' \
        'PC : 0x003CE210' >"$test_work/failure.log"
    printf '%s\n' 'Trap signal 11 in AFSPLUS19' \
        >"$test_work/hosted-failure.log"
    check_log "$test_work/clean.log"
    if check_log "$test_work/failure.log" >/dev/null 2>&1; then
        echo "AROS serial failure scanner accepted its failure fixture" >&2
        exit 1
    fi
    if check_log "$test_work/hosted-failure.log" >/dev/null 2>&1; then
        echo "AROS serial failure scanner accepted its Hosted failure fixture" >&2
        exit 1
    fi
    echo "aros-serial-log self-test=PASS"
    exit 0
fi

if [ "$#" -ne 1 ]; then
    echo "usage: $0 SERIAL-OR-STDOUT-LOG" >&2
    exit 64
fi

check_log "$1"
