#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause

# Build a self-contained, host-checked MacAROS Alpha-0 qualification package.
# This script never installs into or starts a MacAROS tree.

set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
output=${AFSPLUS_AROS_PACKAGE_OUTPUT:-"$repo_root/build/aros-alpha0"}
aros_build=${AROS_BUILD:-"$HOME/aros-build"}
aros_crosstools=${AROS_CROSSTOOLS:-"$HOME/aros-crosstools"}
sdk="$aros_build/bin/darwin-aarch64"
aros_clang="$aros_crosstools/bin/clang"
developer="$sdk/AROS/Developer"
staging=$(mktemp -d "${TMPDIR:-/tmp}/afsplus-aros-package.XXXXXX")

cleanup() {
    if [ -n "$staging" ] && [ -d "$staging" ]; then
        rm -r "$staging"
    fi
}
trap cleanup EXIT HUP INT TERM

[ ! -e "$output" ] || {
    echo "Refusing to replace existing package: $output" >&2
    exit 73
}
[ -x "$aros_clang" ] || {
    echo "Missing AROS compiler: $aros_clang" >&2
    exit 69
}
[ -x "$sdk/tools/collect-aros" ] || {
    echo "Missing AROS build tree: $sdk" >&2
    exit 69
}
[ -f "$developer/lib/startup.o" ] || {
    echo "Missing AROS program startup: $developer/lib/startup.o" >&2
    exit 69
}

cd "$repo_root"

echo "[aros-package] qualify and materialize the complete handler"
AFSPLUS_AROS_HANDLER_OUTPUT="$staging/afsplus-handler" \
    tools/check-aros-ffi.sh

echo "[aros-package] build the target-side Alpha-0 probe"
COMPILER_PATH="$sdk/tools:$aros_crosstools/bin" \
    "$aros_clang" --target=aarch64-unknown-aros \
    -mcmodel=large -ffixed-x18 -O2 -std=gnu11 \
    -Wall -Wextra -Wconversion -Wsign-conversion -Werror \
    -Wno-pointer-sign \
    -isystem "$developer/include" \
    -isystem "$sdk/gen/include" \
    -isystem "$sdk/gen/include/aros/posixc" \
    -isystem "$developer/include/aros/stdc" \
    -nostartfiles -nodefaultlibs \
    -L "$developer/lib" -L "$aros_crosstools/lib/generic" \
    "$developer/lib/startup.o" native/aros/tests/alpha0_probe.c \
    -o "$staging/AFSPlusAlpha0Probe" \
    -Wl,--allow-multiple-definition -Wl,--start-group \
    -lpthread -lposixc -lstdc -lstdcio -ldos -lexec -laros \
    -lautoinit -llibinit -lutility -lamiga -larossupport \
    -Wl,--end-group -lclang_rt.builtins-aarch64
chmod 755 "$staging/AFSPlusAlpha0Probe"

echo "[aros-package] create and verify the 64 MiB image"
cargo run --quiet --release -p afsplus-core --bin afsplus-mkfs -- \
    --size-mib 64 --label AFSPlusAlpha0 "$staging/Unit19"
cargo run --quiet --release -p afsplus-check -- \
    "$staging/Unit19" --json >"$staging/check-before.json"

cp native/aros/AFSPLUS19.mountlist "$staging/AFSPLUS19"
cp docs/aros-alpha0-package.md "$staging/README.md"
(
    cd "$staging"
    shasum -a 256 afsplus-handler AFSPlusAlpha0Probe \
        AFSPLUS19 Unit19 check-before.json README.md >SHA256SUMS
)

mkdir -p "$(dirname -- "$output")"
mv "$staging" "$output"
staging=

echo "[aros-package] PASS: $output"
echo "[aros-package] no MacAROS tree was modified"
