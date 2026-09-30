#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""Compact a closed AROS ELF64 module without dropping its loader relocations.

LoadSeg resolves relocations by symbol section/value, not by symbol name.
Keep a full original for diagnosis; only the compact output belongs in L:.
"""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile

HEADER = struct.Struct('<16sHHIQQQIHHHHHH')
SECTION = struct.Struct('<IIQQQQIIQQ')
SYMBOL = struct.Struct('<IBBHQQ')
RELA = struct.Struct('<QQq')
KEEP = {'afsplus_Handler', 'handler', 'afsplus_aros_mount',
        'afsplus_aros_packet_process'}


class Module:
    def __init__(self, path):
        self.data = Path(path).read_bytes()
        self.header = HEADER.unpack_from(self.data)
        ident, kind = self.header[:2]
        if ident[:6] != b'\x7fELF\x02\x01' or ident[7:9] != b'\x0f\x01' or kind != 1:
            raise ValueError('expected little-endian AROS ABI1 ELF64 ET_REL')
        if self.header[11] != SECTION.size:
            raise ValueError('unexpected section-header size')
        self.sections = [SECTION.unpack_from(self.data, self.header[6] + i * SECTION.size)
                         for i in range(self.header[12])]
        strings = self.payload(self.sections[self.header[13]])
        self.names = [self.string(strings, s[0]) for s in self.sections]
        if len(set(self.names)) != len(self.names):
            raise ValueError('duplicate section names cannot be compared unambiguously')
        self.symbols = {}
        for index, section in enumerate(self.sections):
            if section[1] != 2:
                continue
            if section[9] != SYMBOL.size or section[5] % SYMBOL.size:
                raise ValueError('unexpected symbol table layout')
            strings = self.payload(self.sections[section[6]])
            self.symbols[index] = [(self.string(strings, name), info, other, shndx, value, size)
                                  for name, info, other, shndx, value, size
                                  in SYMBOL.iter_unpack(self.payload(section))]
        for symbols in self.symbols.values():
            for name, _, _, section, _, _ in symbols[1:]:
                if section == 0:
                    raise ValueError(f'closed module has undefined symbol: {name}')

    @staticmethod
    def string(data, offset):
        end = data.index(b'\0', offset)
        return data[offset:end].decode('utf-8')

    def payload(self, section):
        return b'' if section[1] == 8 else self.data[section[4]:section[4] + section[5]]

    def section_identity(self, index):
        return self.names[index] if 0 < index < len(self.sections) else index

    def signature(self):
        allocated = []
        relocations = []
        for index, section in enumerate(self.sections):
            if section[2] & 2:
                allocated.append((self.names[index], section[1:4], section[5], section[8],
                                  hashlib.sha256(self.payload(section)).hexdigest()))
            if section[1] == 9:
                raise ValueError('SHT_REL is not supported; expected explicit-addend RELA')
            if section[1] != 4 or not self.sections[section[7]][2] & 2:
                continue
            if section[9] != RELA.size:
                raise ValueError('unexpected relocation layout')
            for offset, info, addend in RELA.iter_unpack(self.payload(section)):
                name, binding, visibility, target, value, size = self.symbols[section[6]][info >> 32]
                relocations.append((self.names[section[7]], offset, info & 0xffffffff, addend,
                                    binding, visibility, self.section_identity(target), value, size))
        entries = Counter((name, info, other, self.section_identity(section), value, size)
                          for symbols in self.symbols.values()
                          for name, info, other, section, value, size in symbols if name in KEEP)
        if {entry[0] for entry in entries} != KEEP:
            raise ValueError('missing required handler symbols')
        return sorted(allocated), Counter(relocations), entries


def compact(source, output, objcopy, source_revision=None, debug_output=None):
    source, output = Path(source), Path(output)
    original = Module(source)
    expected = original.signature()
    debug = Path(debug_output) if debug_output else Path(str(output) + '.debug')
    report = Path(str(output) + '.size.json')
    for path in (output, debug, report):
        if path.exists():
            raise ValueError(f'refusing to overwrite {path}')
    output.parent.mkdir(parents=True, exist_ok=True)
    names = sorted({s[0] for symbols in original.symbols.values() for s in symbols} - KEEP - {''})
    # Short names stay human-distinguishable; the companion has their full names.
    prefix = '$afs$'
    while any(name.startswith(prefix) for name in names):
        prefix += '$'
    with tempfile.TemporaryDirectory() as work:
        mapping = Path(work) / 'symbols'
        if any(any(c.isspace() for c in name) for name in names):
            raise ValueError('symbol name contains whitespace')
        mapping.write_text(''.join(f'{name} {prefix}{index:x}\n' for index, name in enumerate(names)))
        candidate = Path(work) / 'handler'
        command = [str(objcopy), '--strip-debug', '--strip-unneeded',
                   *[f'--keep-symbol={name}' for name in sorted(KEEP)],
                   f'--redefine-syms={mapping}', str(source), str(candidate)]
        subprocess.run(command, check=True)
        compacted = Module(candidate)
        if (compacted.header[:6] != original.header[:6]
                or compacted.header[7] != original.header[7]
                or compacted.signature() != expected):
            raise ValueError('compaction changed allocated bytes, entry symbols or relocations')
        debug.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, debug)
        shutil.copy2(candidate, output)
    output.chmod(0o755)
    result = {
        'source_revision': source_revision,
        'source': str(source.resolve()),
        'debug': debug.name if debug.parent == output.parent else str(debug.resolve()),
        'output_basename': output.name,
        'source_sha256': hashlib.sha256(original.data).hexdigest(),
        'output_sha256': hashlib.sha256(compacted.data).hexdigest(),
        'source_bytes': len(original.data), 'output_bytes': len(compacted.data),
        'saved_bytes': len(original.data) - len(compacted.data),
        'allocated_sections_unchanged': True, 'relocations_unchanged': True,
        'relocation_count': sum(expected[1].values()),
        'allocated_bytes': sum(s[5] for s in original.sections if s[2] & 2),
        'bss_bytes': sum(s[5] for s in original.sections if s[1] == 8 and s[2] & 2),
        'source_symbol_metadata_bytes': sum(s[5] for s in original.sections if s[1] in (2, 3)),
        'output_symbol_metadata_bytes': sum(s[5] for s in compacted.sections if s[1] in (2, 3)),
        'retained_entry_symbols': sorted(KEEP),
        'objcopy_version': subprocess.check_output([str(objcopy), '--version'], text=True).strip(),
        'symbol_names_compacted': True,
        'debug_artifact_required_for_symbolication': True,
        'native_runtime_verified': False,
    }
    report.write_text(json.dumps(result, indent=2) + '\n')
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--objcopy', required=True, type=Path)
    parser.add_argument('--source-revision')
    parser.add_argument('--debug-output', type=Path)
    args = parser.parse_args()
    print(json.dumps(compact(args.source, args.output, args.objcopy, args.source_revision, args.debug_output), indent=2))


if __name__ == '__main__':
    main()
