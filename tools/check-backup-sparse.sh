#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause
set -eu
repo=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
work=$(mktemp -d "${TMPDIR:-/tmp}/afsplus-sparse.XXXXXX")
trap 'rm -R -- "$work"' EXIT HUP INT TERM
cargo run --quiet --manifest-path "$repo/Cargo.toml" -p afsplus-backup --example sparse-fixture >"$work/archive.tar"
python3 - "$work/archive.tar" <<'PY'
import pathlib, sys, tarfile
path = pathlib.Path(sys.argv[1])
assert path.stat().st_size < 16384
with tarfile.open(path) as archive:
    for name, size in [('mixed', 1048583), ('holes', 64*1024*1024), ('empty', 0)]:
        item = archive.getmember('files/' + name)
        assert item.size == size and item.mtime == 42.125
        stream = archive.extractfile(item)
        if name == 'mixed':
            expected = bytearray(size)
            expected[4096:4101] = b'hello'
            assert stream.read() == expected
        elif size:
            assert stream.read(16) == bytes(16)
            stream.seek(size - 16)
            assert stream.read() == bytes(16)
        else:
            assert stream.read() == b''
print('sparse_interop python=PASS mixed=PASS all-hole=PASS empty=PASS')
PY
mkdir "$work/out"
bsdtar --version
bsdtar -xf "$work/archive.tar" -C "$work/out"
python3 - "$work/out/files" <<'PY'
import pathlib, sys
root = pathlib.Path(sys.argv[1])
expected = bytearray(1048583)
expected[4096:4101] = b'hello'
assert (root / 'mixed').read_bytes() == expected
assert (root / 'empty').read_bytes() == b''
holes = root / 'holes'
assert holes.stat().st_size == 64*1024*1024
assert holes.stat().st_blocks * 512 < holes.stat().st_size
with holes.open('rb') as stream:
    assert stream.read(16) == bytes(16)
    stream.seek(-16, 2)
    assert stream.read() == bytes(16)
print('sparse_interop bsdtar=PASS content=PASS hole-expansion=NO preservation=CONTENT-ONLY')
PY
cargo run --quiet --manifest-path "$repo/Cargo.toml" -p afsplus-backup --example sparse-fixture -- --headers >"$work/headers"
python3 - "$work/headers" <<'PY'
import pathlib, sys, tarfile
wire = pathlib.Path(sys.argv[1]).read_bytes()
assert len(wire) == 3 * 512
for i, size in enumerate([(1 << 33) - 1, 1 << 33, (1 << 64) - 1]):
    block = wire[i*512:(i+1)*512]
    assert tarfile.TarInfo.frombuf(block, 'utf-8', 'strict').size == size
    assert block[124:136] == tarfile.itn(size, 12, tarfile.GNU_FORMAT)
print('sparse_interop python-wide-size-codec=PASS payload-qualification=SEPARATE')
PY
