#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""Reject AROS AArch64 handler artifacts that violate the external ABI."""

from __future__ import annotations

import argparse
from pathlib import Path
import re
import struct
import subprocess
import sys


ELF_HEADER = struct.Struct("<16sHHIQQQIHHHHHH")
ELFCLASS64 = 2
ELFDATA2LSB = 1
ELFOSABI_AROS = 15
ELF_ABI_VERSION = 1
ET_REL = 1
EM_AARCH64 = 183
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
    except AbiError as error:
        print(f"aros-aarch64-abi result=FAIL reason={error}", file=sys.stderr)
        return 1
    print(
        "aros-aarch64-abi result=PASS "
        f"file={args.artifact.name} format=ET_REL osabi=AROS "
        "machine=AArch64 x18=0 tpidr=0"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
