#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause
set -eu
repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
[ "$#" = 1 ] || { echo "usage: $0 NEW-OUTPUT" >&2; exit 64; }
output=$1
[ ! -e "$output" ] || { echo "refusing existing output: $output" >&2; exit 73; }
. "$repo_root/native/aros/profiles/darwin-aarch64.sh"
COMPILER_PATH="$aros_sdk/tools:$AROS_CROSSTOOLS/bin" \
    "$aros_cc" --target="$aros_target" $aros_arch_flags \
    -O2 -std=gnu11 -Wall -Wextra -Werror -Wno-pointer-sign \
    $aros_program_includes -nostartfiles -nodefaultlibs $aros_lib_dirs \
    "$aros_startup" "$repo_root/native/aros/tests/kill_probe.c" \
    -o "$output" -Wl,--allow-multiple-definition -Wl,--start-group -lstdc -lstdcio -ldos -lexec -laros \
    -lautoinit -llibinit -lutility -lamiga -larossupport \
    -Wl,--end-group -lclang_rt.builtins-aarch64
