#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause

# The AFS+ handler on native AROS x86_64, in QEMU.
#
# The x86_64 package compiles, links and audits clean, and until this gate
# nothing had ever run it. Here an AFS+ volume is made on the host, put on a
# raw second disk, and mounted by the x86_64 handler on a pc-x86_64 AROS
# booted from the nightly ISO. The Alpha-0 probe, the DOS probe with its
# STEADY round, the tour and the driver probe run on the machine itself,
# every line leaves through the second serial port, and the host judges: each
# probe's verdict, no FAIL anywhere, and afsplus-check on the image after
# QEMU has exited.
#
# AFSPLUS_QEMU_CONTROL=blank gives the same run a zeroed image. The mount has
# to fail and this gate has to fail with it; a gate that passes on a blank
# disk is proving nothing.
#
# Needs: the x86_64 profile's toolchain (native/aros/profiles/x86_64.sh), the
# nightly pc-x86_64 boot ISO, qemu-system-x86_64, xorriso and bsdtar.
#
# This gate never touches the Hosted MacAROS instance: QEMU is its own
# machine and nothing here runs aros-ctl.

set -u

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
output=${AFSPLUS_QEMU_OUTPUT:-"$repo_root/build/qemu-aros-x86_64"}
control=${AFSPLUS_QEMU_CONTROL:-none}
# leak: the probe keeps a lock per round, and STEADY has to see the heap grow.
steady_mode=
if [ "$control" = leak ]; then steady_mode=" LEAK"; fi
iso=${AFSPLUS_QEMU_ISO:-$(ls "$HOME"/aros-native/AROS-*-pc-x86_64-boot-iso/aros-pc-x86_64.iso 2>/dev/null | tail -1)}
boot_timeout=${AFSPLUS_QEMU_TIMEOUT:-900}
# The Control line of the DOSDriver. COMMIT stays at its default of five
# seconds, the delayed commit a person gets. CACHE=BUFFERS is named because
# of what STEADY measures: its heap clause reads the handler's heap after a
# warm-up and again after the measured rounds, and calls a difference a leak.
# With CACHE=AUTO the handler sizes its read cache from free memory, and on
# this machine, with a gigabyte free, the cache is still filling long after
# the warm-up ends: the run below it reads 1,840 bytes more heap over 100
# rounds with the peak flat, and the same run with CACHE=BUFFERS reads 1,128
# bytes less. The cache is not per-operation state, so BUFFERS is the mount
# in which that clause measures what it claims to.
mount_control=${AFSPLUS_QEMU_MOUNT_CONTROL:-CACHE=BUFFERS}
memory=${AFSPLUS_QEMU_MEMORY:-1024}
work=$(mktemp -d "${TMPDIR:-/tmp}/afsplus-qemu-x86_64.XXXXXX")
qemu_pid=

cleanup() {
    status=$?
    if [ -n "$qemu_pid" ]; then
        kill "$qemu_pid" 2>/dev/null
        wait "$qemu_pid" 2>/dev/null
    fi
    if [ "$status" -ne 0 ] && [ "${AFSPLUS_QEMU_KEEP_FAILURE:-0}" = 1 ]; then
        echo "[qemu-x86_64] keeping $work" >&2
    else
        rm -rf "$work"
    fi
}
trap cleanup EXIT
trap 'exit 130' HUP INT TERM

case "$control" in
none|blank|leak) ;;
*) echo "AFSPLUS_QEMU_CONTROL must be none, blank or leak" >&2; exit 64 ;;
esac
[ ! -e "$output" ] || {
    echo "[qemu-x86_64] refusing to replace existing result: $output" >&2
    exit 73
}
[ -f "$iso" ] || {
    echo "[qemu-x86_64] no pc-x86_64 boot ISO: $iso" >&2
    exit 69
}
for tool in qemu-system-x86_64 xorriso bsdtar; do
    command -v "$tool" >/dev/null || {
        echo "[qemu-x86_64] $tool not found" >&2
        exit 69
    }
done

checks=0
fails=0
ok() {
    checks=$((checks + 1))
    if [ "$1" -eq 0 ]; then
        echo "  ok   $2"
    else
        fails=$((fails + 1))
        echo "  FAIL $2"
    fi
}
has() { LC_ALL=C grep -a -q -- "$2" "$1" 2>/dev/null; }

result="$work/result"
mkdir -p "$result"
cd "$repo_root"

# ---- the package and the probes, built by the x86_64 profile -------------

echo "[qemu-x86_64] 1: the x86_64 package, with the probes"
AFSPLUS_AROS_DIST_PROBES=1 tools/package-aros-dist.sh x86_64 "$work/dist" \
    >"$work/dist.log" 2>&1
ok $? "the handler, the four programs and the two probes build for x86_64"
[ "$fails" -eq 0 ] || { tail -30 "$work/dist.log" >&2; exit 1; }
cp "$work/dist/abi-report.txt" "$work/dist/build-profile.txt" "$result/"

# ---- the volume, made and checked on the host ----------------------------

echo "[qemu-x86_64] 2: a 64 MiB AFS+ volume on the host"
image="$work/afsplus.img"
cargo run --quiet --release -p afsplus-tools --bin mkafsplus -- \
    --profile workstation --size-mib 64 --label AFSPlusAlpha0 \
    --case-insensitive "$image"
ok $? "mkafsplus made the image"
cargo run --quiet --release -p afsplus-check --bin afsplus-check -- \
    "$image" --json >"$result/check-before.json"
has "$result/check-before.json" '"clean":true'
ok $? "afsplus-check: clean before the run"
if [ "$control" = blank ]; then
    # The control keeps the size and loses the volume: the handler has to
    # refuse a disk that carries no AFS+ superblock.
    dd if=/dev/zero of="$image" bs=1048576 count=64 >/dev/null 2>&1
    echo "  control: the image is zeroed; the mount must fail"
fi
# The driver probe writes, so it gets a disk of its own rather than the
# volume it would otherwise corrupt.
scratch="$work/scratch.img"
dd if=/dev/zero of="$scratch" bs=1048576 count=8 >/dev/null 2>&1

# ---- the ISO, remastered ------------------------------------------------

echo "[qemu-x86_64] 3: the nightly ISO with the handler, the tools and a startup"
tree="$work/iso"
mkdir -p "$tree"
bsdtar -xf "$iso" -C "$tree"
ok $? "the nightly ISO is unpacked (nothing under ~/aros-native is touched)"
chmod -R u+w "$tree"
cp "$work/dist/L/afsplus-handler" "$tree/L/afsplus-handler"
cp "$work/dist/C"/* "$work/dist/Probes"/* "$tree/C/"

# The DOSDriver describes the whole raw second disk. It is kept out of
# DEVS:DOSDrivers because the system's own Startup-Sequence mounts everything
# in that drawer before S:User-Startup runs, and the mount this gate wants to
# see is its own, with its own exit code.
#
# The geometry counts in the volume's own 4096-byte blocks (de_SizeBlock is longwords, so 1024), one
# block per cylinder, and the handler turns a block number into a byte offset
# on the device; ata.device's 512-byte sectors divide 4096, which is the only
# thing the block-device contract asks of them.
cat >"$tree/Devs/AFSPLUS19" <<'EOF'
FileSystem      = afsplus-handler
Device          = ata.device
Unit            = 0
Flags           = 0
Surfaces        = 1
BlocksPerTrack  = 1
LowCyl          = 0
HighCyl         = 16383
Reserved        = 0
BlockSize       = 4096
Buffers         = 64
BufMemType      = 1
Mask            = 0
StackSize       = 262144
Priority        = 5
GlobVec         = -1
DosType         = 0x4146532b
Activate        = 0
EOF
[ -z "$mount_control" ] || printf 'Control = "%s"\n' "$mount_control" \
    >>"$tree/Devs/AFSPLUS19"
cp "$tree/Devs/AFSPLUS19" "$result/AFSPLUS19"

steps=""
step() {  # step <name> <command...>
    name=$1
    shift
    steps="$steps $name"
    printf '%s >RAM:out/%s.o\nEcho "$RC" >RAM:out/%s.rc\n' "$*" "$name" "$name"
}
{
    echo 'FailAt 21'
    echo 'MakeDir RAM:out'
    step mount Mount DEVS:AFSPLUS19
    step info AFSPlusInfo AFSPLUS19:
    step alpha0 AFSPlusAlpha0Probe
    step tour AFSPlusTour AFSPLUS19:
    step dos AFSPlusDosProbe
    step steady AFSPlusDosProbe STEADY 100$steady_mode
    step list List AFSPLUS19: ALL
    step packets AFSPlusInfo AFSPLUS19: PACKETS
    step driver AFSPlusDriverProbe ata.device 1 16 8 WRITE
    step shutdown Mount AFSPLUS19: SHUTDOWN
    for name in $steps; do
        for ext in o rc; do
            printf 'Echo "==BEGIN %s.%s==" >SER1:\nType RAM:out/%s.%s >SER1:\nEcho "==END==" >SER1:\n' \
                "$name" "$ext" "$name" "$ext"
        done
    done
    echo 'Echo "==AFSPLUS-QEMU-DONE==" >SER1:'
    echo 'Echo "==AFSPLUS-QEMU-DONE==" >DEBUG:'
} >"$tree/S/User-Startup"
cp "$tree/S/User-Startup" "$result/User-Startup"

# Boot straight through, and put the kernel's own diagnostics on the first
# serial port, where a handler that never starts leaves its trace.
sed -i '' \
    's/^set timeout=5/set timeout=0/; s|multiboot2 /boot/pc/bootstrap.xz ATA=32bit \$bootstrap_flags |multiboot2 /boot/pc/bootstrap.xz ATA=32bit debug=serial $bootstrap_flags |' \
    "$tree/boot/grub/grub.cfg"
xorriso -as mkisofs -R -J -V AROS -o "$work/test.iso" \
    -b boot/grub/i386-pc/eltorito.img -no-emul-boot -boot-load-size 4 \
    -boot-info-table --grub2-boot-info "$tree" >"$work/xorriso.log" 2>&1
ok $? "the test ISO is built"
rm -rf "$tree"

# ---- the run -------------------------------------------------------------

echo "[qemu-x86_64] 4-5: pc-x86_64 AROS in QEMU, one boot"
started=$(date +%s)
qemu-system-x86_64 -m "$memory" -cdrom "$work/test.iso" -boot d \
    -drive file="$image",format=raw,if=ide,index=0 \
    -drive file="$scratch",format=raw,if=ide,index=1 \
    -display none -no-reboot \
    -serial file:"$work/com1.log" -serial file:"$work/com2.log" \
    >"$work/qemu.log" 2>&1 &
qemu_pid=$!
waited=0
while [ "$waited" -lt "$boot_timeout" ] \
    && ! LC_ALL=C grep -a -q 'AFSPLUS-QEMU-DONE' "$work/com2.log" 2>/dev/null
do
    kill -0 "$qemu_pid" 2>/dev/null || break
    sleep 5
    waited=$((waited + 5))
done
kill "$qemu_pid" 2>/dev/null
wait "$qemu_pid" 2>/dev/null
qemu_pid=
elapsed=$(( $(date +%s) - started ))
cp "$work/com1.log" "$result/serial-debug.log" 2>/dev/null
cp "$work/com2.log" "$result/serial-out.log" 2>/dev/null
has "$work/com2.log" 'AFSPLUS-QEMU-DONE'
ok $? "the startup ran to its end (${elapsed}s, waited ${waited}s)"
echo "boot_to_done_seconds=$elapsed" >"$result/timing.txt"

out="$work/out"
mkdir -p "$out"
LC_ALL=C tr -d '\r' <"$work/com2.log" | awk -v dir="$out" '
    /^==BEGIN / { name = $2; sub(/==$/, "", name); file = dir "/" name; printf "" > file; next }
    /^==END==/  { file = ""; next }
    file != ""  { print >> file }'
cp -R "$out" "$result/out"

code() { tr -d ' \r\n' <"$out/$1.rc" 2>/dev/null; }
exits() {
    [ "$(code "$1")" = "$2" ]
    ok $? "$3: \$RC $2 (got $(code "$1"))"
}
# A probe prints its verdict and may print a FAIL after it, as STEADY does
# when its bound is crossed; a FAIL anywhere is a failure, as on Hosted.
no_failure() {
    if LC_ALL=C grep -a -q "^\[$2\] FAIL" "$out/$1" 2>/dev/null; then
        LC_ALL=C grep -a "^\[$2\] FAIL" "$out/$1" >&2
        return 1
    fi
    return 0
}

# ---- the verdict ---------------------------------------------------------

echo "[qemu-x86_64] 6: what the machine said"
# Mount registers the DOSDriver and returns; with Activate 0 the handler is
# started by the first program that touches AFSPLUS19:, so the line below
# says only that DOS took the mountlist. That the x86_64 handler then ran
# and served the volume is what AFSPlusInfo's answer proves, and it is the
# check the blank-image control turns red.
exits mount 0 "Mount took the DOSDriver"
has "$out/info.o" '"schema"'
ok $? "the x86_64 handler served the volume: AFSPlusInfo answered through the extension packet"

no_failure alpha0.o AFSPLUS-ALPHA0 && has "$out/alpha0.o" '^\[AFSPLUS-ALPHA0\] PASS'
ok $? "the Alpha-0 probe passes on x86_64"
exits alpha0 0 "the Alpha-0 probe"

no_failure tour.o AFSPLUS-TOUR && has "$out/tour.o" '^\[AFSPLUS-TOUR\] PASS$'
ok $? "the tour of clones, watches and attributes passes"

no_failure dos.o AFSPLUS-DOS && has "$out/dos.o" '^\[AFSPLUS-DOS\] PASS '
ok $? "the DOS semantics probe passes"
has "$out/dos.o" '^\[AFSPLUS-DOS\] v2 watch taken 1 then 0, removed$'
ok $? "the watch was taken and removed"

no_failure steady.o AFSPLUS-DOS && has "$out/steady.o" '^\[AFSPLUS-DOS\] STEADY rounds 100 '
ok $? "STEADY 100: a hundred rounds cost the system nothing"
has "$out/steady.o" '^\[AFSPLUS-DOS\] STEADY heap before [1-9][0-9]* '
ok $? "and the heap was read before and after"

! has "$out/list.o" 'dosprobe'
ok $? "the probes left no drawer behind"
has "$out/packets.o" '^packet 28 [1-9][0-9]* '
ok $? "the comment travelled as ACTION_SET_COMMENT, not an emulation"

no_failure driver.o AFSPLUS-DRIVER && has "$out/driver.o" '^\[AFSPLUS-DRIVER\] PASS$'
ok $? "ata.device meets the block-device contract"

exits shutdown 0 "the dismount and flush"

echo "[qemu-x86_64] 7: the image, after QEMU has exited"
cargo run --quiet --release -p afsplus-check --bin afsplus-check -- \
    "$image" --json >"$result/check-after.json" 2>"$work/check-after.err"
has "$result/check-after.json" '"clean":true'
ok $? "afsplus-check: clean after the run"

(
    cd "$result"
    find . -type f ! -name SHA256SUMS | sed 's|^\./||' | sort \
        | xargs shasum -a 256 >SHA256SUMS
)

echo
echo "[qemu-x86_64] $checks checks, $fails failures, ${elapsed}s from boot to done"
if [ "$fails" -ne 0 ]; then
    for file in "$out"/*.o; do
        [ -e "$file" ] || continue
        echo "--- $(basename -- "$file")"
        LC_ALL=C cat "$file"
    done
    echo "--- the last of the kernel's serial debug"
    LC_ALL=C tail -c 3000 "$work/com1.log" 2>/dev/null
fi

mkdir -p "$(dirname -- "$output")"
mv "$result" "$output"
if [ "$fails" -ne 0 ]; then
    if [ "$control" = leak ]; then
        echo "[qemu-x86_64] CONTROL FAIL, as it must be when STEADY is the failure:"
        grep -h "heap held\|FAIL STEADY" "$output"/* 2>/dev/null | head -3
    elif [ "$control" = blank ]; then
        echo "[qemu-x86_64] CONTROL FAIL, as it must be: a zeroed image is not"
        echo "[qemu-x86_64] an AFS+ volume and the gate says so. Evidence: $output"
    else
        echo "[qemu-x86_64] FAIL: $output"
    fi
    exit 1
fi
if [ "$control" = leak ]; then
    echo "[qemu-x86_64] CONTROL PASSED, which is the defect: a lock leaked per" >&2
    echo "[qemu-x86_64] round and STEADY saw nothing." >&2
    exit 1
fi
if [ "$control" = blank ]; then
    echo "[qemu-x86_64] CONTROL PASSED, which is the defect: the gate accepted"
    echo "[qemu-x86_64] a zeroed disk as an AFS+ volume." >&2
    exit 1
fi
echo "[qemu-x86_64] PASS: $output"
