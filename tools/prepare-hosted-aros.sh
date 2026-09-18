#!/bin/sh
# Bring a freshly built Hosted AROS tree, and the fork clone it was built
# from, to the state the AFS+ Hosted gates need. Everything here is outside
# what `make` in an AROS build tree produces by itself, and a rebuild of the
# tree loses all of it. Idempotent: run it after every build.
#
#   AROS_BUILD       the build tree (default ~/aros-build)
#   AROS_SOURCE      the fork clone (default ../aros-upstream beside afsplus)
#   AROS_APPLE_CORE  the sibling clone Mount comes from
#   AROS_CROSSTOOLS  the AROS cross toolchain (default ~/aros-crosstools)
#   AROS_BUILD_TOOLS the host build tools (default ~/aros-build-tools)
set -e

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
aros_build=${AROS_BUILD:-"$HOME/aros-build"}
aros_source=${AROS_SOURCE:-"$repo_root/../aros-upstream"}
apple_core=${AROS_APPLE_CORE:-"$repo_root/../aros-apple-core"}
crosstools=${AROS_CROSSTOOLS:-"$HOME/aros-crosstools"}
build_tools=${AROS_BUILD_TOOLS:-"$HOME/aros-build-tools"}
tree=$aros_build/bin/darwin-aarch64/AROS

for dir in "$aros_build" "$aros_source" "$apple_core" "$crosstools"; do
    [ -d "$dir" ] || { echo "missing: $dir" >&2; exit 1; }
done

# The crosstools must be on PATH for a build, and must NOT be on the PATH of
# a gate run, or the gate's host compiles pick up the AROS clang.
PATH=$crosstools/bin:$build_tools:/opt/homebrew/bin:$PATH
export PATH

step() { echo "[prepare] $*"; }

# (1) The one local change in the fork clone. Without it fdsk.device expunges
# itself during its first open, because CreateNewProc's speculative MEMF_31BIT
# allocation runs the low-memory handlers, and every gate dies with SIGILL.
patch_file=$aros_source/rom/dos/createnewproc.c
if grep -q MEMF_NO_EXPUNGE "$patch_file"; then
    step "createnewproc.c already carries the no-expunge change"
else
    step "applying native/aros/upstream/dos-createnewproc-noexpunge.patch"
    (cd "$aros_source" && patch -p1 \
        < "$repo_root/native/aros/upstream/dos-createnewproc-noexpunge.patch")
    (cd "$aros_build" && make kernel-dos)
fi

# (2) Modules the system metatarget does not build but the gates load.
[ -f "$tree/Libs/posixc.library" ] || { step "building posixc.library"
    (cd "$aros_build" && make compiler-posixc); }
[ -f "$tree/Devs/fdsk.device" ] || { step "building fdsk.device"
    (cd "$aros_build" && make workbench-devs-fdsk); }
# The Fast File System handler, the baseline of the benchmark runner.
[ -f "$tree/L/afs-handler" ] || { step "building afs-handler"
    (cd "$aros_build" && make kernel-fs-afs); }

# (3) Directories nothing creates. AROS/S must exist or the first boot hangs
# with no startup script; DiskImages is where FDSK: is assigned.
step "directories"
mkdir -p "$tree/S" "$tree/DiskImages"

# (4) C:Mount with the SHUTDOWN switch, which the gates use to stop a handler
# without dismounting it. The stock command of the fork clone has no such
# switch; the sibling aros-apple-core tree has it. Built through the fork's
# own rule with that one file put in place and taken out again on every exit
# path, so neither clone keeps a change, and with the stock command kept
# beside the result as C:Mount.stock.
if strings "$tree/C/Mount" 2>/dev/null | grep -q 'SHUTDOWN/S'; then
    step "C:Mount already understands SHUTDOWN"
else
    step "building C:Mount from aros-apple-core"
    mount_src=$aros_source/workbench/c/Mount.c
    mount_saved=$(mktemp)
    cp "$mount_src" "$mount_saved"
    trap 'cp "$mount_saved" "$mount_src"; rm -f "$mount_saved"' EXIT INT TERM
    [ -f "$tree/C/Mount.stock" ] || cp "$tree/C/Mount" "$tree/C/Mount.stock"
    cp "$apple_core/workbench/c/Mount.c" "$mount_src"
    (cd "$aros_build" && make workbench-c)
    cp "$mount_saved" "$mount_src"
    rm -f "$mount_saved"
    trap - EXIT INT TERM
    strings "$tree/C/Mount" | grep -q 'SHUTDOWN/S'
fi

step "checking what a gate will look for"
for path in "$tree/Libs/posixc.library" "$tree/Devs/fdsk.device" \
        "$tree/L/afs-handler" \
        "$tree/C/Mount" "$tree/S" "$tree/DiskImages"; do
    [ -e "$path" ] || { echo "still missing: $path" >&2; exit 1; }
done
grep -q MEMF_NO_EXPUNGE "$patch_file"
strings "$tree/C/Mount" | grep -q 'SHUTDOWN/S'
echo "[prepare] PASS: $tree is ready for the Hosted gates"
