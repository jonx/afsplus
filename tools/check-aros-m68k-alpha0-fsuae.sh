#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause

# Build the external AFS+ handler with the qualified experimental Rust/m68k
# toolchain, run the Alpha-0 operation matrix under native AROS in FS-UAE, then
# replay every deterministic intent-log cut through the same handler.

set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
boot_adf=${AFSPLUS_AROS_M68K_BOOT_ADF:-}
system_iso=${AFSPLUS_AROS_M68K_SYSTEM_ISO:-}
fs_uae=${AFSPLUS_FS_UAE:-fs-uae}
model=${AFSPLUS_AROS_M68K_MODEL:-A4000/040}
cpu_speed=${AFSPLUS_AROS_M68K_CPU_SPEED:-real}
fast_memory=${AFSPLUS_AROS_M68K_FAST_MEMORY:-8192}
zorro_iii_memory=${AFSPLUS_AROS_M68K_ZORRO_III_MEMORY:-65536}
serial_port_base=${AFSPLUS_AROS_M68K_SERIAL_PORT_BASE:-24600}
output=${AFSPLUS_AROS_M68K_ALPHA0_OUTPUT:-"$repo_root/build/aros-m68k-alpha0-fsuae"}
m68k_build=${AROS_M68K_BUILD:-"$HOME/aros-m68k-build"}
macaros_root=${MACAROS_ROOT:-"$repo_root/../Macaros"}
rust_toolchain=${AFSPLUS_AROS_M68K_RUST_TOOLCHAIN:-m68k-ccr-fixed}
target_cargo=${AFSPLUS_AROS_M68K_CARGO:-cargo}
target_json=${AFSPLUS_AROS_M68K_RUST_TARGET_JSON:-"$macaros_root/hosted/rust/m68k-unknown-aros.json"}
llvm_lib=${AFSPLUS_AROS_M68K_LLVM_LIB:-}
trace_startup=${AFSPLUS_AROS_M68K_TRACE_STARTUP:-0}

require_file() {
    [ -f "$1" ] || { echo "Missing required file: $1" >&2; exit 66; }
}

require_executable() {
    command -v "$1" >/dev/null 2>&1 || {
        echo "Missing required executable: $1" >&2
        exit 69
    }
}

[ -n "$boot_adf" ] || {
    echo "Set AFSPLUS_AROS_M68K_BOOT_ADF to bootdisk-amiga-m68k.adf" >&2
    exit 64
}
[ -n "$system_iso" ] || {
    echo "Set AFSPLUS_AROS_M68K_SYSTEM_ISO to the matching aros-amiga-m68k.iso" >&2
    exit 64
}
[ ! -e "$output" ] || {
    echo "Refusing to replace existing m68k evidence: $output" >&2
    exit 73
}

sdk="$m68k_build/bin/amiga-m68k"
host_root="$m68k_build/bin/darwin-aarch64"
host_tools="$host_root/tools"
cc="$host_tools/crosstools/m68k-aros-gcc"
collect_aros="$host_tools/collect-aros"
genmodule="$host_tools/genmodule"
nm_tool="$host_tools/crosstools/m68k-aros-nm"
objdump_tool="$host_tools/crosstools/m68k-aros-objdump"
include="$sdk/AROS/Developer/include"
gen_include="$sdk/gen/include"
stdc_include="$include/aros/stdc"
posixc_include="$gen_include/aros/posixc"
target_lib="$sdk/AROS/Developer/lib"
gcc_lib="$host_tools/crosstools/lib/gcc/m68k-aros/6.5.0"

for file in \
    "$boot_adf" "$system_iso" "$target_json" \
    "$repo_root/native/aros/afsplus.conf" \
    "$repo_root/native/aros/afsplus-handler-autolibs.patch" \
    "$repo_root/native/aros/AFSPLUS19-m68k.mountlist" \
    "$repo_root/native/aros/tests/m68k-alpha0-sequence" \
    "$repo_root/native/aros/tests/m68k-replay-old-sequence" \
    "$repo_root/native/aros/tests/m68k-replay-new-sequence" \
    "$macaros_root/hosted/rust/aros_fs_glue.c" \
    "$macaros_root/hosted/rust/aros_env_glue.c"
do
    require_file "$file"
done
for executable in "$fs_uae" "$cc" "$collect_aros" "$genmodule" \
    "$nm_tool" "$objdump_tool" "$target_cargo" cargo rustup bsdtar nc patch \
    shasum
do
    require_executable "$executable"
done
if command -v gtimeout >/dev/null 2>&1; then
    timeout_command=gtimeout
elif command -v timeout >/dev/null 2>&1; then
    timeout_command=timeout
else
    echo "Missing required executable: gtimeout or timeout" >&2
    exit 69
fi
rustup run "$rust_toolchain" rustc -vV >/dev/null 2>&1 || {
    echo "Rust toolchain is not registered: $rust_toolchain" >&2
    exit 69
}
target_cpu=$(sed -n 's/^[[:space:]]*"cpu":[[:space:]]*"\([^"]*\)".*/\1/p' \
    "$target_json")
if [ "$target_cpu" = M68000 ] && [ -z "$llvm_lib" ]; then
    echo "M68000 qualification requires AFSPLUS_AROS_M68K_LLVM_LIB" >&2
    exit 64
fi
if [ -n "$llvm_lib" ]; then
    require_file "$llvm_lib/libLLVM.dylib"
    # macOS strips DYLD_* while launching a #!/bin/sh script through its
    # platform shell.  Export it from inside the script so the selected rustc
    # actually loads the qualified experimental backend.
    DYLD_LIBRARY_PATH="$llvm_lib${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}"
    export DYLD_LIBRARY_PATH
fi

work=$(mktemp -d /tmp/afsplus-m68k-alpha0.XXXXXX)
result="$work/result"
system="$work/system"
build="$work/handler"
fixtures="$work/fixtures"
mkdir -p "$result/cases" "$system" "$build/module/include"

cleanup() {
    status=$?
    if [ "$status" -ne 0 ] && [ "${AFSPLUS_KEEP_M68K_FAILURE:-0}" = 1 ]; then
        echo "[m68k-alpha0] keeping failed work directory: $work" >&2
        return
    fi
    [ ! -d "$work" ] || rm -r "$work"
}
trap cleanup EXIT HUP INT TERM

cd "$repo_root"
echo "[m68k-alpha0] extract matching official AROS system media"
bsdtar -xf "$system_iso" -C "$system"
chmod -R u+w "$system"
require_file "$system/boot/amiga/aros-rom.bin"
require_file "$system/boot/amiga/aros-ext.bin"
require_file "$system/S/Startup-Sequence"
mkdir -p "$system/DiskImages" "$system/Devs/DOSDrivers"

echo "[m68k-alpha0] build Rust static library with patched m68k backend"
if [ "$target_cargo" = cargo ]; then
    CARGO_TARGET_DIR="$work/rust-target" \
    CARGO_PROFILE_RELEASE_OPT_LEVEL=2 \
    CARGO_PROFILE_RELEASE_LTO=false \
    CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 \
        cargo "+$rust_toolchain" build -p afsplus-aros-ffi --release \
            --target "$target_json" -Zjson-target-spec \
            -Zbuild-std=std,panic_abort
else
    target_sysroot=$(rustup run "$rust_toolchain" rustc --print sysroot)
    RUSTC="$target_sysroot/bin/rustc" \
    RUSTDOC="$target_sysroot/bin/rustdoc" \
    CARGO_TARGET_DIR="$work/rust-target" \
    CARGO_PROFILE_RELEASE_OPT_LEVEL=2 \
    CARGO_PROFILE_RELEASE_LTO=false \
    CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 \
        "$target_cargo" build -p afsplus-aros-ffi --release \
            --target "$target_json" -Zjson-target-spec \
            -Zbuild-std=std,panic_abort
fi
target_name=$(basename "$target_json" .json)
archive="$work/rust-target/$target_name/release/libafsplus_aros_ffi.a"
require_file "$archive"

echo "[m68k-alpha0] generate and compile native handler module"
"$genmodule" -c "$repo_root/native/aros/afsplus.conf" -d "$build/module" \
    writelibdefs afsplus handler
"$genmodule" -c "$repo_root/native/aros/afsplus.conf" -d "$build/module" \
    writefiles afsplus handler
patch -s "$build/module/afsplus_start.c" \
    "$repo_root/native/aros/afsplus-handler-autolibs.patch"

"$cc" -m68000 -O2 -std=gnu11 -Wall -Wextra -Werror \
    -Wno-volatile-register-var -D__NOLIBBASE__ \
    -DAFSPLUS_AROS_TRACE_STARTUP="$trace_startup" \
    -I "$stdc_include" -I "$include" -I "$gen_include" \
    -I "$repo_root/api" -I "$repo_root/native/aros" \
    -c "$repo_root/native/aros/afsplus_handler.c" -o "$build/handler.o"
"$cc" -m68000 -O2 -std=gnu11 -Wall -Wextra -Werror \
    -Wno-volatile-register-var \
    -DAFSPLUS_AROS_TRACE_STARTUP="$trace_startup" \
    -I "$stdc_include" -I "$include" -I "$gen_include" \
    -I "$repo_root/api" -I "$repo_root/native/aros" \
    -c "$repo_root/native/aros/afsplus_packet.c" -o "$build/packet.o"
"$cc" -m68000 -O2 -std=gnu11 -Wall -Wextra -Werror \
    -Wno-volatile-register-var \
    -I "$stdc_include" -I "$include" -I "$gen_include" \
    -I "$repo_root/api" -I "$repo_root/native/aros" \
    -c "$repo_root/native/aros/afsplus_trackdisk.c" -o "$build/trackdisk.o"
for source in afsplus_start afsplus_end; do
    "$cc" -m68000 -O2 -std=gnu11 -Wall -Wextra -Werror \
        -Wno-volatile-register-var -Wno-missing-field-initializers \
        -Wno-unused-parameter -Wno-pointer-sign \
        -D__AROS__ -D__NOLIBBASE__ \
        -I "$include" -I "$gen_include" -I "$build/module" \
        -c "$build/module/$source.c" -o "$build/module/$source.o"
done
for glue in aros_fs_glue aros_env_glue; do
    "$cc" -m68000 -O2 -Wno-pointer-sign \
        -I "$gen_include" -I "$include" -I "$posixc_include" \
        -I "$stdc_include" \
        -c "$macaros_root/hosted/rust/$glue.c" \
        -o "$build/module/$glue.o"
done

handler="$build/afsplus-handler"
COMPILER_PATH="$host_tools/crosstools/bin" \
    "$collect_aros" --eh-frame-hdr --allow-multiple-definition \
    -L"$target_lib" -L"$gcc_lib" -o "$handler" \
    "$build/module/afsplus_start.o" "$build/handler.o" \
    "$build/packet.o" "$build/trackdisk.o" \
    "$build/module/aros_fs_glue.o" "$build/module/aros_env_glue.o" \
    "$archive" "$build/module/afsplus_end.o" \
    -\( -lstdc.static -lamiga -larossupport -laros -ldos -lutility \
    -llibinit -lautoinit -lposixc -lstdcio -lstdc -lexec -lpthread \
    -lgcc -\)
if "$nm_tool" --undefined-only "$handler" | grep -Eq '[^[:space:]]'; then
    echo "Unresolved symbol in m68k handler:" >&2
    "$nm_tool" --undefined-only "$handler" >&2
    exit 65
fi
"$objdump_tool" -d "$handler" >"$build/handler.disasm"
if [ "$target_cpu" = M68000 ] && grep -Eiq \
    'mulu\.l|muls\.l|mulul|mulsl|\.short[[:space:]]+0x4c[0-3][0-9a-f]' \
        "$build/handler.disasm"; then
    echo "M68020 long-multiply instruction in M68000 handler:" >&2
    grep -Ein \
        'mulu\.l|muls\.l|mulul|mulsl|\.short[[:space:]]+0x4c[0-3][0-9a-f]' \
        "$build/handler.disasm" | head -20 >&2
    exit 65
fi
if [ "$trace_startup" = 0 ] && strings "$handler" | grep -Eq \
    '\[AFSPLUS\] packet|\[AFSPLUS-CORE\]|\[AFSPLUS-RUST\]|\[AFSPLUS-AROS\]|\[AFSPLUS-STARTUP\]'; then
    echo "Diagnostic trace string leaked into m68k handler" >&2
    exit 65
fi

echo "[m68k-alpha0] build target operation and replay probes"
"$cc" -m68000 -O2 -std=gnu11 -Wall -Wextra -Werror \
    -Wno-volatile-register-var -Wno-pointer-sign \
    "$repo_root/native/aros/tests/alpha0_probe.c" \
    -o "$build/AFSPlusAlpha0Probe"
"$cc" -m68000 -O2 -std=gnu11 -Wall -Wextra -Werror \
    -Wno-volatile-register-var -Wno-pointer-sign \
    "$repo_root/native/aros/tests/replay_probe.c" \
    -o "$build/AFSPlusReplayProbe"
"$cc" -m68000 -O2 -std=gnu11 -Wall -Wextra -Werror \
    -Wno-volatile-register-var -Wno-pointer-sign \
    "$repo_root/native/aros/tests/m68k_route_probe.c" \
    -o "$build/AFSPlusRouteProbe"
cp "$handler" "$system/L/afsplus-handler"
cp "$build/AFSPlusAlpha0Probe" "$system/C/AFSPlusAlpha0Probe"
cp "$build/AFSPlusReplayProbe" "$system/C/AFSPlusReplayProbe"
cp "$build/AFSPlusRouteProbe" "$system/C/AFSPlusRouteProbe"
cp "$repo_root/native/aros/AFSPLUS19-m68k.mountlist" \
    "$system/Devs/DOSDrivers/AFSPLUS19"
chmod 755 "$system/L/afsplus-handler" "$system/C/AFSPlusAlpha0Probe" \
    "$system/C/AFSPlusReplayProbe" "$system/C/AFSPlusRouteProbe"

run_guest() {
    case_name=$1
    sequence=$2
    source_image=$3
    port=$4
    case_result="$result/cases/$case_name"
    mkdir -p "$case_result/host"
    cp "$source_image" "$system/DiskImages/Unit19"
    cp "$sequence" "$system/S/Startup-Sequence"

    "$timeout_command" "${TIMEOUT:-90}" "$fs_uae" \
        --amiga-model="$model" \
        --kickstart-file="$system/boot/amiga/aros-rom.bin" \
        --kickstart-ext-file="$system/boot/amiga/aros-ext.bin" \
        --floppy-drive-0="$boot_adf" \
        --hard-drive-0="$system" \
        --hard-drive-0-label='AROS Live CD' \
        --hard-drive-1="$case_result/host" \
        --hard-drive-1-label=HOST \
        --cpu-speed="$cpu_speed" \
        --fast-memory="$fast_memory" --zorro-iii-memory="$zorro_iii_memory" \
        --serial-port="tcp://127.0.0.1:$port/wait" \
        --joystick-port-0=none --joystick-port-1=none \
        --log-file="$case_result/fs-uae.log" \
        >"$case_result/fs-uae.stdout" 2>&1 &
    emulator_pid=$!
    sleep 3
    nc -d 127.0.0.1 "$port" >"$case_result/serial.log" &
    serial_pid=$!
    set +e
    wait "$emulator_pid"
    emulator_status=$?
    wait "$serial_pid"
    set -e
    [ "$emulator_status" -eq 0 ] || {
        echo "FS-UAE case $case_name failed with status $emulator_status" >&2
        exit 1
    }
    require_file "$case_result/host/mount.status"
    grep -qx pass "$case_result/host/mount.status"
}

echo "[m68k-alpha0] run clean operation matrix"
alpha_image="$work/Unit19.alpha0"
cargo run --quiet --release -p afsplus-core --bin afsplus-mkfs -- \
    --size-mib 64 --label AFSPlusAlpha0 --case-insensitive "$alpha_image"
run_guest alpha0 "$repo_root/native/aros/tests/m68k-alpha0-sequence" \
    "$alpha_image" "$serial_port_base"
alpha_case="$result/cases/alpha0"
grep -qx pass "$alpha_case/host/route.status"
grep -qxF '[AFSPLUS-ROUTE] PASS' "$alpha_case/host/route.out"
grep -qx pass "$alpha_case/host/probe.status"
grep -qxF \
    '[AFSPLUS-ALPHA0] PASS create/read/write/truncate/rename/fsync/casefold' \
    "$alpha_case/host/probe.out"
[ "$(cat "$alpha_case/host/alpha0.from-aros")" = hello ]
cargo run --quiet --release -p afsplus-check --bin afsplus-check -- \
    "$system/DiskImages/Unit19" --json >"$alpha_case/check-after.json"
grep -q '"clean":true' "$alpha_case/check-after.json"
grep -q '"log_records_pending":0' "$alpha_case/check-after.json"
cp "$system/DiskImages/Unit19" "$result/Unit19.alpha0.final"

echo "[m68k-alpha0] generate and replay deterministic crash cuts"
cargo run --quiet --release -p afsplus-check \
    --bin afsplus-crash-fixtures -- "$fixtures"
cp "$fixtures/manifest.tsv" "$fixtures/README.txt" "$result/"
tab=$(printf '\t')
index=0
while IFS="$tab" read -r fixture expected pending_before description; do
    [ "$fixture" != fixture ] || continue
    case_name=${fixture%.img}
    case_result="$result/cases/$case_name"
    echo "[m68k-alpha0] replay $case_name -> $expected"
    cargo run --quiet --release -p afsplus-check --bin afsplus-check -- \
        "$fixtures/$fixture" --json >"$work/check-before.json"
    grep -q '"clean":true' "$work/check-before.json"
    grep -q "\"log_records_pending\":$pending_before" \
        "$work/check-before.json"
    run_guest "$case_name" \
        "$repo_root/native/aros/tests/m68k-replay-$expected-sequence" \
        "$fixtures/$fixture" "$((serial_port_base + index + 1))"
    mv "$work/check-before.json" "$case_result/check-before.json"
    grep -qx pass "$case_result/host/replay.status"
    grep -qx "\[AFSPLUS-REPLAY\] PASS expected=$expected" \
        "$case_result/host/replay.out"
    cargo run --quiet --release -p afsplus-check --bin afsplus-check -- \
        "$system/DiskImages/Unit19" --json >"$case_result/check-after.json"
    grep -q '"clean":true' "$case_result/check-after.json"
    grep -q '"log_records_pending":0' "$case_result/check-after.json"
    if [ "$expected" = old ]; then expected_generation=2; else expected_generation=3; fi
    grep -q "\"generation\":$expected_generation" "$case_result/check-after.json"
    {
        echo "expected=$expected"
        echo "pending_before=$pending_before"
        echo "description=$description"
        printf 'fixture_sha256='
        shasum -a 256 "$fixtures/$fixture" | awk '{print $1}'
        printf 'after_sha256='
        shasum -a 256 "$system/DiskImages/Unit19" | awk '{print $1}'
    } >"$case_result/report.txt"
    index=$((index + 1))
done <"$fixtures/manifest.tsv"
[ "$index" -eq 6 ]

profile=m68020-or-newer-reference-engine
plain_68000_claim=none
if [ "$target_cpu" = M68000 ]; then
    profile=m68000-reference-engine
    if [ "$model" = A500 ] && [ "$zorro_iii_memory" = 0 ]; then
        profile=m68000-a500-emulator
        plain_68000_claim=emulator-runtime
    fi
fi
cp "$handler" "$result/afsplus-handler"

{
    echo "format=afsplus-aros-m68k-alpha0-fsuae-v1"
    echo "result=PASS"
    echo "alpha0=PASS"
    echo "replay_cases=$index"
    echo "profile=$profile"
    echo "model=$model"
    echo "cpu_speed=$cpu_speed"
    echo "target_cpu=$target_cpu"
    echo "fast_memory=$fast_memory"
    echo "zorro_iii_memory=$zorro_iii_memory"
    echo "hardware_claim=none"
    echo "physical_a500_claim=none"
    echo "plain_68000_claim=$plain_68000_claim"
    echo "performance_claim=none"
    echo "trace_startup=$trace_startup"
    echo "rust_toolchain=$rust_toolchain"
    if [ -n "$llvm_lib" ]; then
        printf 'llvm_dylib_sha256='
        shasum -a 256 "$llvm_lib/libLLVM.dylib" | awk '{print $1}'
    else
        echo "llvm_dylib_sha256=toolchain-default"
    fi
    printf 'rustc='
    rustup run "$rust_toolchain" rustc -V | awk 'NR == 1 {print; exit}'
    printf 'target_json_sha256='
    shasum -a 256 "$target_json" | awk '{print $1}'
    printf 'handler_sha256='
    shasum -a 256 "$handler" | awk '{print $1}'
    printf 'boot_adf_sha256='
    shasum -a 256 "$boot_adf" | awk '{print $1}'
    printf 'system_iso_sha256='
    shasum -a 256 "$system_iso" | awk '{print $1}'
    printf 'rom_sha256='
    shasum -a 256 "$system/boot/amiga/aros-rom.bin" | awk '{print $1}'
    printf 'ext_rom_sha256='
    shasum -a 256 "$system/boot/amiga/aros-ext.bin" | awk '{print $1}'
    printf 'fs_uae_version='
    "$fs_uae" --version 2>&1 | awk 'NR == 1 {print; exit}'
} >"$result/report.txt"

(
    cd "$result"
    find . -type f ! -name SHA256SUMS -print | LC_ALL=C sort | \
        xargs shasum -a 256 >SHA256SUMS
)
mkdir -p "$(dirname -- "$output")"
mv "$result" "$output"
echo "aros-m68k-alpha0-fsuae result=PASS evidence=$output"
