#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause

# Fail when an AROS diagnostic stream contains a modal software-failure
# requester.  Such a requester can leave the emulator alive until its timeout,
# so process status alone is not a sufficient guest verdict.

set -eu

check_log() {
    log=$1
    [ -f "$log" ] || {
        echo "Missing AROS diagnostic log: $log" >&2
        return 66
    }

    failure_pattern='Software Failure!|Guru Meditation'
    if LC_ALL=C grep -Eq "$failure_pattern" "$log"; then
        echo "AROS software-failure requester detected in $log:" >&2
        LC_ALL=C grep -En \
            'Software Failure!|Guru Meditation|Task[[:space:]]*:|Error:|PC[[:space:]]*:|Module |Function ' \
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
    check_log "$test_work/clean.log"
    if check_log "$test_work/failure.log" >/dev/null 2>&1; then
        echo "AROS serial failure scanner accepted its failure fixture" >&2
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
