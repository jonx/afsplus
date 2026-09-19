#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause
#
# The third-party probe kit: portable/probe/afsplus_probe.c recognises an
# AFS+ volume from its first 4 KiB, and this gate proves it on the test
# vectors of docs/18 section 6, each made by the project's own tools:
#
#   empty, one file, Unicode names, a large sparse file, a large directory,
#   symlinks and hard links, journal replay cases (crash fixtures), corrupt
#   metadata cases (the corruption corpus), an unknown feature bit,
#   and the negatives: a foreign block, a damaged identification block,
#   a short read.
#
# For every AFS+ vector the probe's UUID, label, block size and block count
# must equal what afsplus-info reads through the full reader. The kit is
# written to $AFSPLUS_PROBE_KIT (default build/probe-kit): the images, the
# probe's answer and afsplus-info's report for each, for a third party to
# take. Exit 0 only when every expectation holds.
#
# Needs a C99 compiler, the release tools (cargo build --release) and python3.

set -u
repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"
target=${CARGO_TARGET_DIR:-"$repo_root/target"}
bin="$target/release"
kit=${AFSPLUS_PROBE_KIT:-"$repo_root/build/probe-kit"}
cc=${CC:-cc}

for t in mkafsplus afsplus-info afsplus-populate afsplus-crash-fixtures afsplus-corruption-corpus; do
    [ -x "$bin/$t" ] || { echo "probe-kit: missing $bin/$t; run cargo build --release for afsplus-tools, afsplus-core and afsplus-check" >&2; exit 69; }
done
rm -rf "$kit"
mkdir -p "$kit/vectors" "$kit/expected"
checks=0; fails=0
ok() { checks=$((checks + 1)); if [ "$1" -ne 0 ]; then fails=$((fails + 1)); echo "  FAIL $2"; fi; }

# 1. The probe, compiled as a third party would, with the strictest flags.
$cc -std=c99 -Wall -Wextra -Werror -pedantic -DAFSPLUS_PROBE_MAIN \
    -o "$kit/afsplus-probe" portable/probe/afsplus_probe.c || { echo "probe-kit: the probe does not compile"; exit 1; }
ok 0 "the probe compiles under -std=c99 -Wall -Wextra -Werror -pedantic"
probe="$kit/afsplus-probe"

# 2. The AFS+ vectors.
work=$(mktemp -d "${TMPDIR:-/tmp}/probe-kit.XXXXXX")
trap 'rm -rf "$work"' EXIT
mk() { "$bin/mkafsplus" --size-mib "$2" --label "$3" "$kit/vectors/$1.afsp" > /dev/null || exit 1; }
populate() { "$bin/afsplus-populate" "$kit/vectors/$1.afsp" "$2" > /dev/null || exit 1; }

mk empty 8 Empty
mk one-file 8 OneFile
mkdir -p "$work/one"; printf 'hello\n' > "$work/one/hello.txt"; populate one-file "$work/one"
mk unicode 8 "Ünïcödé ✓"
mkdir -p "$work/uni"; printf 'a\n' > "$work/uni/Ärger.txt"; printf 'b\n' > "$work/uni/日本語.txt"; printf 'c\n' > "$work/uni/emoji 😀.txt"; populate unicode "$work/uni"
mk sparse 64 Sparse
mkdir -p "$work/sparse"; python3 -c "
f=open('$work/sparse/big.bin','wb'); f.seek(40*1024*1024); f.write(b'end'); f.close()"; populate sparse "$work/sparse"
mk large-dir 32 LargeDir
mkdir -p "$work/dir"; python3 -c "
import os
for i in range(3000): open(os.path.join('$work/dir', 'file-%04d.txt' % i), 'w').write(str(i))"; populate large-dir "$work/dir"
mk links 8 Links
mkdir -p "$work/links"; printf 'target\n' > "$work/links/target.txt"; ln -s target.txt "$work/links/soft"; ln "$work/links/target.txt" "$work/links/hard"; populate links "$work/links"
"$bin/afsplus-crash-fixtures" "$work/crash" > /dev/null
for f in "$work"/crash/*.img; do cp "$f" "$kit/vectors/journal-$(basename "$f" .img).afsp"; done
"$bin/afsplus-corruption-corpus" "$work/corrupt" > /dev/null
for f in "$work"/corrupt/*.img; do cp "$f" "$kit/vectors/corrupt-$(basename "$f" .img).afsp"; done

# An unknown feature bit: bit 40 of incompat, set in the identification
# block and the block resealed, as a future epoch-1 feature would appear.
cp "$kit/vectors/empty.afsp" "$kit/vectors/unknown-feature.afsp"
python3 - "$kit/vectors/unknown-feature.afsp" <<'PY'
import sys
p = sys.argv[1]
b = bytearray(open(p, 'rb').read(4096))
off = 32 + 153
v = int.from_bytes(b[off:off+8], 'little') | (1 << 40)
b[off:off+8] = v.to_bytes(8, 'little')
def crc32c(data, crc=0xFFFFFFFF):
    for byte in data:
        crc ^= byte
        for _ in range(8):
            crc = (crc >> 1) ^ (0x82F63B78 & -(crc & 1))
    return crc
c = crc32c(b[:28]); c = crc32c(b'\0\0\0\0', c); c = crc32c(b[32:], c)
b[28:32] = ((~c) & 0xFFFFFFFF).to_bytes(4, 'little')
with open(p, 'r+b') as f: f.write(b)
PY

# 3. The negatives.
python3 -c "open('$kit/vectors/foreign.img','wb').write(b'DOS\x01' + b'\0' * 4092)"
cp "$kit/vectors/empty.afsp" "$kit/vectors/damaged.afsp"
python3 -c "
p='$kit/vectors/damaged.afsp'; b=bytearray(open(p,'rb').read()); b[100]^=1; open(p,'wb').write(b)"
head -c 100 "$kit/vectors/empty.afsp" > "$kit/vectors/short.img"

# 4. Expectations. Every AFS+ vector: probe OK and equal to afsplus-info.
agree() {  # agree <vector>: the probe and afsplus-info name the same volume
    v=$1
    "$probe" --json "$kit/vectors/$v.afsp" > "$kit/expected/$v.probe.json" 2>&1
    rc=$?
    "$bin/afsplus-info" "$kit/vectors/$v.afsp" --json > "$kit/expected/$v.info.json" 2>/dev/null || true
    [ "$rc" -eq 0 ] && python3 - "$kit/expected/$v.probe.json" "$kit/expected/$v.info.json" <<'PY'
import json, sys
p = json.load(open(sys.argv[1])); i = json.load(open(sys.argv[2]))['volume']
uuid = i['uuid']; uuid = '-'.join((uuid[:8], uuid[8:12], uuid[12:16], uuid[16:20], uuid[20:]))
same = (p['afsplus'] and p['uuid'] == uuid and p['label'] == i['label']
        and p['block_size'] == i['block_size'] and p['total_blocks'] == i['total_blocks']
        and p['features'] == {k: i['features'][k] for k in ('compat', 'ro_compat', 'incompat')})
sys.exit(0 if same else 1)
PY
}
for v in empty one-file unicode sparse large-dir links; do
    agree "$v"; ok $? "$v: the probe names the volume as afsplus-info does"
done
"$probe" "$kit/vectors/unicode.afsp" | grep -q 'Ünïcödé ✓'; ok $? "the Unicode label comes through byte for byte"
for f in "$kit"/vectors/journal-*.afsp; do
    n=$(basename "$f" .afsp)
    "$probe" --json "$f" > "$kit/expected/$n.probe.json"; ok $? "$n: a crash image is still recognised (the identification block is never rewritten)"
done
for f in "$kit"/vectors/corrupt-*.afsp; do
    n=$(basename "$f" .afsp)
    "$probe" --json "$f" > "$kit/expected/$n.probe.json"; rc=$?
    case "$n" in
        corrupt-ident-checksum)   # the corpus damages the identification block itself
            [ "$rc" -eq 1 ] && grep -q '"status":2' "$kit/expected/$n.probe.json"; ok $? "$n: a damaged identification block is reported as damaged, not as foreign" ;;
        corrupt-ident-unknown-incompat)   # bit 63 set and resealed by the corpus
            [ "$rc" -eq 0 ] && grep -q '"incompat":"0x8000000000000003"' "$kit/expected/$n.probe.json"; ok $? "$n: the corpus's unknown feature bit is reported" ;;
        *) ok $rc "$n: a volume with damaged metadata elsewhere is still recognised" ;;
    esac
done
"$probe" --json "$kit/vectors/unknown-feature.afsp" > "$kit/expected/unknown-feature.probe.json" \
    && grep -q '"incompat":"0x0000010000000003"' "$kit/expected/unknown-feature.probe.json"
ok $? "an unknown incompat feature is reported, not refused: a prober identifies, a mounter decides"
"$probe" --json "$kit/vectors/foreign.img" > "$kit/expected/foreign.probe.json"; rc=$?
[ "$rc" -eq 1 ] && grep -q '"status":1' "$kit/expected/foreign.probe.json"; ok $? "a foreign block is not AFS+ (status 1, exit 1)"
"$probe" --json "$kit/vectors/damaged.afsp" > "$kit/expected/damaged.probe.json"; rc=$?
[ "$rc" -eq 1 ] && grep -q '"status":2' "$kit/expected/damaged.probe.json"; ok $? "one flipped byte in the identification block: damaged (status 2), never a yes"
"$probe" --json "$kit/vectors/short.img" > "$kit/expected/short.probe.json"; rc=$?
[ "$rc" -eq 1 ] && grep -q '"status":4' "$kit/expected/short.probe.json"; ok $? "a short read is refused (status 4)"

# 5. The kit's own README.
cat > "$kit/README.md" <<EOF
# AFS+ probe kit

\`afsplus-probe\` recognises an AFS+ volume from its first 4096 bytes:
\`afsplus-probe [--json] <device-or-image>\`, exit 0 for an AFS+ volume.
Source: \`portable/probe/afsplus_probe.{h,c}\` of the AFS+ repository,
BSD-2-Clause, no dependency; \`afsplus_probe()\` is the one function to call.

\`vectors/\` holds the test vectors of docs/18 section 6; \`expected/\` the
probe's answer for each (\`*.probe.json\`) and, for the mountable ones, the
full reader's report (\`*.info.json\`), which the probe must agree with.
EOF
cp portable/probe/afsplus_probe.h portable/probe/afsplus_probe.c "$kit/"

echo "probe-kit: $checks checks, $fails failures; kit in $kit"
[ "$fails" -eq 0 ]
