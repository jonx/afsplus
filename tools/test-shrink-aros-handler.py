#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""Check a real handler pair and prove the equivalence check rejects corruption.

Usage: tools/test-shrink-aros-handler.py original compact
This is a focused release-artifact check, not an emulated/native load claim.
"""
import importlib.util
from pathlib import Path
import struct
import sys
import tempfile

spec = importlib.util.spec_from_file_location('shrink', Path(__file__).with_name('shrink-aros-handler.py'))
shrink = importlib.util.module_from_spec(spec)
spec.loader.exec_module(shrink)
source, output = map(Path, sys.argv[1:])
original, compact = shrink.Module(source), shrink.Module(output)
expected = original.signature()
assert compact.signature() == expected
assert len(compact.data) < len(original.data), 'no size reduction'
with tempfile.TemporaryDirectory() as work:
    damaged = Path(work) / 'bad-handler'
    # A single changed code byte must not pass as harmless metadata stripping.
    text = compact.sections[compact.names.index('.text')]
    data = bytearray(compact.data)
    data[text[4]] ^= 1
    damaged.write_bytes(data)
    assert shrink.Module(damaged).signature() != expected, 'missed altered executable byte'
    # A valid-looking RELA addend change must also be detected independently
    # of section/symbol renumbering performed by objcopy.
    rela = next(s for s in compact.sections if s[1] == 4 and compact.sections[s[7]][2] & 2)
    data = bytearray(compact.data)
    addend = struct.unpack_from('<q', data, rela[4] + 16)[0]
    struct.pack_into('<q', data, rela[4] + 16, addend ^ 1)
    damaged.write_bytes(data)
    assert shrink.Module(damaged).signature() != expected, 'missed altered relocation'
print('PASS: release equivalence; executable-byte and relocation-addend negative controls')
