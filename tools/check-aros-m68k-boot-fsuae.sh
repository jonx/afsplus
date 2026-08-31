#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause

# Prove the native AROS/m68k boot path before adding the AFS+ handler. The
# official boot floppy remains DF0:, while an extracted copy of the matching
# Live CD is exposed as the writable "AROS Live CD:" directory volume. The
# guest writes its verdict to a second HOST: directory volume and shuts down.

set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
boot_adf=${AFSPLUS_AROS_M68K_BOOT_ADF:-}
system_iso=${AFSPLUS_AROS_M68K_SYSTEM_ISO:-}
fs_uae=${AFSPLUS_FS_UAE:-fs-uae}
model=${AFSPLUS_AROS_M68K_MODEL:-A4000/040}
cpu_speed=${AFSPLUS_AROS_M68K_CPU_SPEED:-real}
fast_memory=${AFSPLUS_AROS_M68K_FAST_MEMORY:-8M}
zorro_iii_memory=${AFSPLUS_AROS_M68K_ZORRO_III_MEMORY:-64M}
output=${AFSPLUS_AROS_M68K_OUTPUT:-"$repo_root/build/aros-m68k-boot-fsuae"}

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
    echo "Set AFSPLUS_AROS_M68K_BOOT_ADF to the official bootdisk-amiga-m68k.adf" >&2
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

require_file "$boot_adf"
require_file "$system_iso"
require_file "$repo_root/native/aros/tests/m68k-boot-sequence"
require_executable "$fs_uae"
require_executable bsdtar
if command -v gtimeout >/dev/null 2>&1; then
    timeout_command=gtimeout
elif command -v timeout >/dev/null 2>&1; then
    timeout_command=timeout
else
    echo "Missing required executable: gtimeout or timeout" >&2
    exit 69
fi

mkdir -p "$output/system" "$output/host"
bsdtar -xf "$system_iso" -C "$output/system"
chmod -R u+w "$output/system"
require_file "$output/system/boot/amiga/aros-rom.bin"
require_file "$output/system/boot/amiga/aros-ext.bin"
require_file "$output/system/S/Startup-Sequence"
cp "$repo_root/native/aros/tests/m68k-boot-sequence" \
    "$output/system/S/Startup-Sequence"

set +e
"$timeout_command" 90 "$fs_uae" \
    --amiga-model="$model" \
    --kickstart-file="$output/system/boot/amiga/aros-rom.bin" \
    --uae-kickstart-ext-rom-file="$output/system/boot/amiga/aros-ext.bin" \
    --floppy-drive-0="$boot_adf" \
    --hard-drive-0="$output/system" \
    --hard-drive-0-label='AROS Live CD' \
    --hard-drive-1="$output/host" \
    --hard-drive-1-label=HOST \
    --cpu-speed="$cpu_speed" \
    --fast-memory="$fast_memory" \
    --zorro-iii-memory="$zorro_iii_memory" \
    --stdout >"$output/fs-uae.log" 2>&1
emulator_status=$?
set -e

[ "$emulator_status" -eq 0 ] || {
    echo "FS-UAE did not reach guest shutdown (status $emulator_status)" >&2
    exit 1
}
require_file "$output/host/boot.pass"
require_file "$output/host/avail.txt"
require_file "$output/host/version.txt"
grep -qx 'AFSPLUS M68K BOOT PASS' "$output/host/boot.pass" || {
    echo "AROS/m68k boot marker did not match" >&2
    exit 1
}

{
    echo "format=afsplus-aros-m68k-fsuae-v1"
    echo "result=PASS"
    echo "scope=boot-harness-only"
    echo "hardware_claim=none"
    echo "a500_68000_claim=none"
    echo "model=$model"
    echo "cpu_speed=$cpu_speed"
    echo "fast_memory=$fast_memory"
    echo "zorro_iii_memory=$zorro_iii_memory"
    echo "boot_protocol=official-rom-plus-boot-floppy-plus-live-cd-volume"
    printf 'boot_adf_sha256='
    shasum -a 256 "$boot_adf" | awk '{print $1}'
    printf 'system_iso_sha256='
    shasum -a 256 "$system_iso" | awk '{print $1}'
    printf 'rom_sha256='
    shasum -a 256 "$output/system/boot/amiga/aros-rom.bin" | awk '{print $1}'
    printf 'ext_rom_sha256='
    shasum -a 256 "$output/system/boot/amiga/aros-ext.bin" | awk '{print $1}'
    printf 'fs_uae_version='
    "$fs_uae" --version 2>&1 | awk 'NR == 1 {print; exit}'
    printf 'guest_version='
    awk 'NR == 1 {print; exit}' "$output/host/version.txt"
} >"$output/report.txt"

echo "aros-m68k-boot-fsuae result=PASS evidence=$output"
