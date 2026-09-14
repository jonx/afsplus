#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause
set -eu
repo=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
work=$(mktemp -d "${TMPDIR:-/tmp}/afsplus-tar.XXXXXX")
trap 'rm -R -- "$work"' EXIT HUP INT TERM
cargo run --quiet --manifest-path "$repo/Cargo.toml" -p afsplus-backup --example tar-fixture >"$work/archive.tar"
python3 - "$work/archive.tar" "$repo/crates/afsplus-backup/tests/fixtures/python-ustar.bin" <<'PY'
import io
import pathlib
import sys
import tarfile

def verify(data):
    with tarfile.open(fileobj=io.BytesIO(data)) as archive:
        members = archive.getmembers()
        assert len(members) == 1
        item = members[0]
        assert (item.name, item.size, item.mode, item.uid, item.gid, item.mtime,
                item.uname, item.gname) == ('folder/payload', 5, 0o640, 12, 34,
                                          123, 'user', 'group')
        assert archive.extractfile(item).read() == b'hello'

encoded = pathlib.Path(sys.argv[1]).read_bytes()
verify(encoded)
damaged = bytearray(encoded)
damaged[512] ^= 1
try:
    verify(damaged)
except AssertionError:
    pass
else:
    raise AssertionError('negative payload oracle accepted changed bytes')
fixture = io.BytesIO()
with tarfile.open(fileobj=fixture, mode='w', format=tarfile.USTAR_FORMAT) as archive:
    item = tarfile.TarInfo('folder/payload')
    item.size, item.mode, item.uid, item.gid = 5, 0o640, 12, 34
    item.mtime, item.uname, item.gname = 123, 'user', 'group'
    archive.addfile(item, io.BytesIO(b'hello'))
assert fixture.getvalue() == pathlib.Path(sys.argv[2]).read_bytes()
print('tar_interop python=PASS metadata=PASS fixture=PASS negative-payload-oracle=PASS')
PY
bsdtar --version
bsdtar -xOf "$work/archive.tar" folder/payload >"$work/payload"
python3 - "$work/payload" <<'PY'
import pathlib
import sys
assert pathlib.Path(sys.argv[1]).read_bytes() == b'hello'
print('tar_interop bsdtar=PASS ordinary-file-only=YES')
PY
