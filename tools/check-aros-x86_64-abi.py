#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""Reject AROS x86_64 handler artifacts that violate the external ABI.

The AArch64 audit next to this one asks the questions the AArch64 profile
raises: x18 and the architectural TLS registers. x86_64 raises different ones,
so the audit is per profile rather than one script with a switch.

What an AROS x86_64 module must be:

* an ``ET_REL`` ELF64 object with the AROS OSABI and ABI version 1, because
  ``rom/dos/internalloadseg_elf.c`` loads modules, not executables;
* relocated only with the types that loader implements. Anything else is a
  silent wrong address at load time, not a link error;
* free of undefined symbols, because nothing resolves them after the link.
"""

from __future__ import annotations

import argparse
from pathlib import Path
import struct
import sys


ELF_HEADER = struct.Struct("<16sHHIQQQIHHHHHH")
SECTION_HEADER = struct.Struct("<IIQQQQIIQQ")
RELA = struct.Struct("<QQq")
SYMBOL = struct.Struct("<IBBHQQ")

ELFCLASS64 = 2
ELFDATA2LSB = 1
ELFOSABI_AROS = 15
ELF_ABI_VERSION = 1
ET_REL = 1
EM_X86_64 = 62

SHT_RELA = 4
SHT_SYMTAB = 2
SHN_UNDEF = 0

# Every relocation type rom/dos/internalloadseg_elf.c resolves for x86_64.
SUPPORTED_RELOCATIONS = {
    0: "R_X86_64_NONE",
    1: "R_X86_64_64",
    2: "R_X86_64_PC32",
    4: "R_X86_64_PLT32",
    10: "R_X86_64_32",
    11: "R_X86_64_32S",
    24: "R_X86_64_PC64",
    25: "R_X86_64_GOTOFF64",
}


class AbiError(ValueError):
    """The artifact is not a safe external AROS x86_64 module."""


class Module:
    """The parts of an ELF file this audit reads."""

    def __init__(self, path: Path) -> None:
        try:
            self.blob = path.read_bytes()
        except OSError as error:
            raise AbiError(f"cannot read {path}: {error}") from error
        self.path = path
        if len(self.blob) < ELF_HEADER.size:
            raise AbiError(f"{path}: truncated ELF header")
        values = ELF_HEADER.unpack(self.blob[: ELF_HEADER.size])
        (ident, self.elf_type, self.machine, self.version) = values[:4]
        self.section_offset = values[6]
        self.section_size = values[11]
        self.section_count = values[12]
        if ident[:4] != b"\x7fELF":
            raise AbiError(f"{path}: not an ELF artifact")
        if ident[4] != ELFCLASS64 or ident[5] != ELFDATA2LSB:
            raise AbiError(f"{path}: expected 64-bit little-endian ELF")
        if ident[7] != ELFOSABI_AROS or ident[8] != ELF_ABI_VERSION:
            raise AbiError(
                f"{path}: expected AROS OSABI 15 ABI version 1, got "
                f"{ident[7]}/{ident[8]}"
            )
        if self.elf_type != ET_REL or self.machine != EM_X86_64 or \
                self.version != 1:
            raise AbiError(
                f"{path}: expected ET_REL/EM_X86_64/version 1, got "
                f"{self.elf_type}/{self.machine}/{self.version}"
            )
        if self.section_size != SECTION_HEADER.size:
            raise AbiError(f"{path}: unexpected section header size")
        self.sections = []
        for index in range(self.section_count):
            start = self.section_offset + index * self.section_size
            end = start + self.section_size
            if end > len(self.blob):
                raise AbiError(f"{path}: truncated section header table")
            self.sections.append(SECTION_HEADER.unpack(self.blob[start:end]))

    def contents(self, section) -> bytes:
        _, kind, _, _, offset, size = section[:6]
        if kind == 8:  # SHT_NOBITS occupies no file space.
            return b""
        if offset + size > len(self.blob):
            raise AbiError(f"{self.path}: section runs past the end of file")
        return self.blob[offset:offset + size]

    def relocation_types(self) -> dict[int, int]:
        """How many relocations of each type the module carries."""
        counted: dict[int, int] = {}
        for section in self.sections:
            if section[1] != SHT_RELA:
                continue
            data = self.contents(section)
            for at in range(0, len(data) - RELA.size + 1, RELA.size):
                _, info, _ = RELA.unpack(data[at:at + RELA.size])
                kind = info & 0xFFFFFFFF
                counted[kind] = counted.get(kind, 0) + 1
        return counted

    def undefined_symbols(self) -> list[str]:
        names = []
        for section in self.sections:
            if section[1] != SHT_SYMTAB:
                continue
            strings = self.contents(self.sections[section[6]])
            data = self.contents(section)
            for at in range(0, len(data) - SYMBOL.size + 1, SYMBOL.size):
                name, _, _, shndx, _, _ = SYMBOL.unpack(
                    data[at:at + SYMBOL.size]
                )
                if shndx != SHN_UNDEF or name == 0:
                    continue
                end = strings.find(b"\0", name)
                names.append(strings[name:end].decode("utf-8", "replace"))
        return names


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "verify an external AROS x86_64 ET_REL module: header, only the "
            "relocation types the AROS ELF loader resolves, no undefined "
            "symbol"
        )
    )
    # Accepted so every profile's audit takes the same command line; this one
    # reads the ELF file itself and needs no disassembler.
    parser.add_argument("--objdump", type=Path, default=None)
    parser.add_argument("artifact", type=Path)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        module = Module(args.artifact)
        counted = module.relocation_types()
        unsupported = sorted(set(counted) - set(SUPPORTED_RELOCATIONS))
        if unsupported:
            listed = ", ".join(f"type {kind}" for kind in unsupported)
            raise AbiError(
                f"{args.artifact}: relocation the AROS x86_64 ELF loader "
                f"does not resolve: {listed}"
            )
        undefined = module.undefined_symbols()
        if undefined:
            preview = "; ".join(sorted(undefined)[:16])
            if len(undefined) > 16:
                preview += f"; ... ({len(undefined) - 16} more)"
            raise AbiError(
                f"{args.artifact}: undefined symbol in a loadable module: "
                f"{preview}"
            )
    except AbiError as error:
        print(f"aros-x86_64-abi result=FAIL reason={error}", file=sys.stderr)
        return 1
    relocations = ",".join(
        f"{SUPPORTED_RELOCATIONS[kind]}={count}"
        for kind, count in sorted(counted.items())
    ) or "none"
    print(
        "aros-x86_64-abi result=PASS "
        f"file={args.artifact.name} format=ET_REL osabi=AROS "
        f"machine=x86-64 relocations={relocations} undefined=0"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
