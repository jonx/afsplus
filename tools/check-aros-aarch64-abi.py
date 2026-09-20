#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""Reject AROS AArch64 handler artifacts that violate the external ABI.

Besides the x18 and TLS-register questions the AArch64 profile raises, this
audit holds the module's ``.bss`` to a bound. A relocatable module's ``.bss``
costs no file bytes and is taken in full at load, on every machine that has
the handler in ``L/``, mounted volume or not: the AROS static pthread library
once put a 1,841,328-byte thread table there. The bound is what keeps a
library like that from coming back unnoticed.
"""

from __future__ import annotations

import argparse
from pathlib import Path
import re
import struct
import subprocess
import sys


ELF_HEADER = struct.Struct("<16sHHIQQQIHHHHHH")
SECTION_HEADER = struct.Struct("<IIQQQQIIQQ")
SYMBOL = struct.Struct("<IBBHQQ")
ELFCLASS64 = 2
ELFDATA2LSB = 1
ELFOSABI_AROS = 15
ELF_ABI_VERSION = 1
ET_REL = 1
EM_AARCH64 = 183

SHT_SYMTAB = 2
SHT_NOBITS = 8

# What the handler may take at load before a volume is mounted. It holds 596
# bytes; 64 KiB leaves room for a cache descriptor or a table somebody adds on
# purpose, and refuses a whole thread library.
BSS_LIMIT = 64 * 1024
# No single zero-filled object may be larger than this. A total that creeps up
# in kilobytes is a design change; one object of megabytes is a library that
# was linked by accident, and this names it.
BSS_OBJECT_LIMIT = 64 * 1024
X18_RE = re.compile(r"\b[wx]18\b", re.IGNORECASE)
TPIDR_RE = re.compile(r"\btpidr(?:ro|2)?_el0\b", re.IGNORECASE)
INSTRUCTION_RE = re.compile(
    r"^\s*[0-9a-f]+:\s+(?:[0-9a-f]{8}|"
    r"(?:[0-9a-f]{2}\s+){3}[0-9a-f]{2})"
    r"\s+([a-z][a-z0-9.]*)\s*(.*?)\s*$",
    re.IGNORECASE,
)


class AbiError(ValueError):
    """The artifact is not a safe external AROS AArch64 module."""


def validate_elf(path: Path) -> None:
    try:
        with path.open("rb") as stream:
            header = stream.read(ELF_HEADER.size)
    except OSError as error:
        raise AbiError(f"cannot read {path}: {error}") from error
    if len(header) != ELF_HEADER.size:
        raise AbiError(f"{path}: truncated ELF header")
    values = ELF_HEADER.unpack(header)
    ident, elf_type, machine, version = values[:4]
    if ident[:4] != b"\x7fELF":
        raise AbiError(f"{path}: not an ELF artifact")
    if ident[4] != ELFCLASS64 or ident[5] != ELFDATA2LSB:
        raise AbiError(f"{path}: expected 64-bit little-endian ELF")
    if ident[7] != ELFOSABI_AROS or ident[8] != ELF_ABI_VERSION:
        raise AbiError(
            f"{path}: expected AROS OSABI 15 ABI version 1, got "
            f"{ident[7]}/{ident[8]}"
        )
    if elf_type != ET_REL or machine != EM_AARCH64 or version != 1:
        raise AbiError(
            f"{path}: expected ET_REL/EM_AARCH64/version 1, got "
            f"{elf_type}/{machine}/{version}"
        )


def sections(blob: bytes, path: Path) -> tuple[list[tuple], int]:
    values = ELF_HEADER.unpack(blob[: ELF_HEADER.size])
    offset, entry_size, count, names_index = (
        values[6], values[11], values[12], values[13]
    )
    if entry_size != SECTION_HEADER.size:
        raise AbiError(f"{path}: unexpected section header size")
    result = []
    for index in range(count):
        start = offset + index * entry_size
        end = start + entry_size
        if end > len(blob):
            raise AbiError(f"{path}: truncated section header table")
        result.append(SECTION_HEADER.unpack(blob[start:end]))
    return result, names_index


def section_bytes(blob: bytes, section: tuple) -> bytes:
    _, kind, _, _, offset, size = section[:6]
    if kind == SHT_NOBITS:
        return b""
    return blob[offset:offset + size]


def bss_report(path: Path) -> tuple[int, list[tuple[str, int]]]:
    """The module's zero-filled size, and its objects over the object bound."""
    blob = path.read_bytes()
    table, names_index = sections(blob, path)
    names = section_bytes(blob, table[names_index])

    def name_of(section: tuple) -> str:
        start = section[0]
        return names[start:names.find(b"\0", start)].decode("utf-8", "replace")

    zero_filled = {
        index: section for index, section in enumerate(table)
        # sh_flags is the third word; SHF_ALLOC means it is given memory.
        if section[1] == SHT_NOBITS and section[2] & 0x2
    }
    total = sum(section[5] for section in zero_filled.values())
    large = []
    for section in table:
        if section[1] != SHT_SYMTAB:
            continue
        strings = section_bytes(blob, table[section[6]])
        data = section_bytes(blob, section)
        for at in range(0, len(data) - SYMBOL.size + 1, SYMBOL.size):
            name, _, _, shndx, _, size = SYMBOL.unpack(data[at:at + SYMBOL.size])
            if shndx not in zero_filled or size <= BSS_OBJECT_LIMIT:
                continue
            end = strings.find(b"\0", name)
            large.append((strings[name:end].decode("utf-8", "replace"), size))
    large.sort(key=lambda item: -item[1])
    return total, large


def disassemble(path: Path, objdump: Path) -> str:
    try:
        result = subprocess.run(
            [str(objdump), "-d", str(path)],
            check=False,
            capture_output=True,
            text=True,
        )
    except OSError as error:
        raise AbiError(f"cannot execute {objdump}: {error}") from error
    if result.returncode != 0:
        detail = result.stderr.strip() or f"exit status {result.returncode}"
        raise AbiError(f"cannot disassemble {path}: {detail}")
    return result.stdout


def forbidden_instructions(text: str) -> list[str]:
    result = []
    for line in text.splitlines():
        match = INSTRUCTION_RE.match(line)
        if match is None:
            continue
        instruction = f"{match.group(1)} {match.group(2)}".strip()
        if X18_RE.search(instruction) or TPIDR_RE.search(instruction):
            result.append(instruction)
    return result


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "verify an external AROS AArch64 ET_REL module and reject all "
            "x18 or architectural TLS-register instructions"
        )
    )
    parser.add_argument("--objdump", required=True, type=Path)
    parser.add_argument("artifact", type=Path)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        validate_elf(args.artifact)
        forbidden = forbidden_instructions(
            disassemble(args.artifact, args.objdump)
        )
        if forbidden:
            preview = "; ".join(forbidden[:16])
            if len(forbidden) > 16:
                preview += f"; ... ({len(forbidden) - 16} more)"
            raise AbiError(
                f"{args.artifact}: forbidden x18/TPIDR machine code: {preview}"
            )
        zero_filled, large = bss_report(args.artifact)
        if large:
            listed = "; ".join(f"{name} {size} bytes" for name, size in large[:4])
            raise AbiError(
                f"{args.artifact}: zero-filled object above "
                f"{BSS_OBJECT_LIMIT} bytes, taken at load whether or not a "
                f"volume is mounted: {listed}"
            )
        if zero_filled > BSS_LIMIT:
            raise AbiError(
                f"{args.artifact}: .bss is {zero_filled} bytes, above the "
                f"{BSS_LIMIT}-byte bound; that memory is taken at load on "
                "every machine"
            )
    except AbiError as error:
        print(f"aros-aarch64-abi result=FAIL reason={error}", file=sys.stderr)
        return 1
    print(
        "aros-aarch64-abi result=PASS "
        f"file={args.artifact.name} format=ET_REL osabi=AROS "
        f"machine=AArch64 x18=0 tpidr=0 bss={zero_filled}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
