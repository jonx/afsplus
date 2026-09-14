#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause
set -eu
repo=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
work=$(mktemp -d "${TMPDIR:-/tmp}/afsplus-envelope.XXXXXX")
trap 'rm -R -- "$work"' EXIT HUP INT TERM
cargo run --quiet --manifest-path "$repo/Cargo.toml" -p afsplus-backup --example envelope-fixture >"$work/archive.tar"
crypto=${AFSPLUS_OPENSSL:-openssl}
if [ "$crypto" = openssl ] && [ -x /opt/homebrew/opt/openssl@3/bin/openssl ]; then
    crypto=/opt/homebrew/opt/openssl@3/bin/openssl
fi
"$crypto" version
python3 - "$work/archive.tar" "$crypto" <<'PY'
import subprocess
import io
import pathlib
import sys
import tarfile

def records(payload):
    result = {}
    while payload:
        length = int(payload.split(b' ', 1)[0])
        record, payload = payload[:length], payload[length:]
        assert len(record) == length and record[-1:] == b'\n'
        key, value = record.split(b' ', 1)[1][:-1].split(b'=', 1)
        assert key not in result
        result[key] = value
    return result

def verify(data):
    data = bytes(data)
    offset = 0
    headers = []
    while any(data[offset:offset+512]):
        block = data[offset:offset+512]
        assert len(block) == 512
        header = tarfile.TarInfo.frombuf(block, 'utf-8', 'strict')
        payload = data[offset+512:offset+512+header.size]
        assert len(payload) == header.size
        headers.append((offset, header, payload))
        offset += 512 + ((header.size + 511) // 512) * 512
    assert len(data) - offset == 1024 and not any(data[offset:])
    assert len(headers) == 3
    beginning, body, ending = headers
    assert beginning[1].name == '_AROS_BACKUP/begin'
    assert beginning[1].type == tarfile.XGLTYPE and ending[1].type == tarfile.REGTYPE
    assert records(beginning[2]) == {b'AROS.backup.envelope': b'2', b'AROS.backup.algorithm': b'sha512-256'}
    assert ending[1].name == '_AROS_BACKUP/complete.pax'
    control = records(ending[2])
    assert control == {b'AROS.backup.end': b'2', b'AROS.backup.bytes': str(ending[0]).encode(),
                       b'AROS.backup.members': b'1',
                       b'AROS.backup.hash': subprocess.run([sys.argv[2], 'dgst', '-sha512-256', '-binary'], input=data[:ending[0]], capture_output=True, check=True).stdout.hex().encode()}
    assert body[1].name == 'files/folder/payload' and body[2] == b'hello'
    return body[0] + 512

encoded = pathlib.Path(sys.argv[1]).read_bytes()
body_offset = verify(encoded)
damaged = bytearray(encoded)
damaged[body_offset] ^= 1
try:
    verify(damaged)
except AssertionError:
    pass
else:
    raise AssertionError('negative digest oracle accepted modified content')
with tarfile.open(fileobj=io.BytesIO(encoded)) as archive:
    assert archive.getnames() == ['files/folder/payload', '_AROS_BACKUP/complete.pax']
    assert archive.extractfile('files/folder/payload').read() == b'hello'
print('envelope_interop independent-digest=PASS counts=PASS python-recovery=PASS negative=PASS')
PY
bsdtar --version
bsdtar -xOf "$work/archive.tar" files/folder/payload >"$work/payload"
python3 - "$work/payload" <<'PY'
import pathlib
import sys
assert pathlib.Path(sys.argv[1]).read_bytes() == b'hello'
print('envelope_interop bsdtar-recovery=PASS ordinary-file-only=YES')
PY
