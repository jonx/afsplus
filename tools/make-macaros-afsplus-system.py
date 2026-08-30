#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""Build the bounded 16-MiB FAT12 bootstrap for native AFS+ QEMU tests."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
from pathlib import Path, PurePosixPath
import struct
import sys


SECTOR_SIZE = 512
SECTOR_COUNT = 32768
SECTORS_PER_CLUSTER = 16
RESERVED_SECTORS = 1
FAT_COUNT = 2
ROOT_ENTRIES = 224
SECTORS_PER_FAT = 6
MEDIA = 0xF8


def load_fat12_class():
    path = Path(__file__).with_name("inject-fat12-file.py")
    spec = importlib.util.spec_from_file_location("afsplus_fat12_inject", path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {path}")
    module = importlib.util.module_from_spec(spec)
    previous = sys.dont_write_bytecode
    try:
        sys.dont_write_bytecode = True
        spec.loader.exec_module(module)
    finally:
        sys.dont_write_bytecode = previous
    return module.Fat12, module.FatError


Fat12, FatError = load_fat12_class()


def empty_image() -> bytes:
    image = bytearray(SECTOR_COUNT * SECTOR_SIZE)
    boot = memoryview(image)[:SECTOR_SIZE]
    boot[0:3] = b"\xeb\x3c\x90"
    boot[3:11] = b"AROSAFS "
    struct.pack_into(
        "<HBHBHHBHHHII",
        boot,
        11,
        SECTOR_SIZE,
        SECTORS_PER_CLUSTER,
        RESERVED_SECTORS,
        FAT_COUNT,
        ROOT_ENTRIES,
        SECTOR_COUNT,
        MEDIA,
        SECTORS_PER_FAT,
        32,
        2,
        0,
        0,
    )
    boot[38] = 0x29
    struct.pack_into("<I", boot, 39, 0xAF5A2026)
    boot[43:54] = b"AROSAFSBOOT"
    boot[54:62] = b"FAT12   "
    boot[510:512] = b"\x55\xaa"

    fat = bytearray(SECTORS_PER_FAT * SECTOR_SIZE)
    fat[0:3] = bytes((MEDIA, 0xFF, 0xFF))
    for index in range(FAT_COUNT):
        start = (RESERVED_SECTORS + index * SECTORS_PER_FAT) * SECTOR_SIZE
        image[start:start + len(fat)] = fat
    root_sector = RESERVED_SECTORS + FAT_COUNT * SECTORS_PER_FAT
    root = root_sector * SECTOR_SIZE
    image[root:root + 32] = Fat12.entry(b"AROSAFSBOOT", 0x08, 0, 0)
    return bytes(image)


def build(
    abi_probe: bytes,
    device: bytes,
) -> bytes:
    files = (
        ("AROS.boot", b"apple-aarch64\n"),
        ("B2PASS.TXT", b"AROS Apple Silicon B2.8 boot volume\n"),
        ("ABIPROBE", abi_probe),
        ("DEVS/afsram.device", device),
    )
    image = Fat12(empty_image())
    for name, content in files:
        image.inject(PurePosixPath(name), content)
    return bytes(image.data)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--abi-probe", required=True, type=Path)
    parser.add_argument("--device", required=True, type=Path)
    parser.add_argument("--verify", action="store_true")
    parser.add_argument("output", type=Path)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        expected = build(
            args.abi_probe.read_bytes(),
            args.device.read_bytes(),
        )
        if args.verify:
            if args.output.read_bytes() != expected:
                raise FatError("native AFS+ FAT12 system image mismatch")
        else:
            if args.output.exists():
                raise FatError(f"refusing to replace {args.output}")
            args.output.write_bytes(expected)
    except (FatError, OSError, RuntimeError) as error:
        print(f"macaros-afsplus-system result=FAIL reason={error}", file=sys.stderr)
        return 1
    print(
        "macaros-afsplus-system result=PASS "
        f"path={args.output} bytes={len(expected)} files=4 "
        f"sha256={hashlib.sha256(expected).hexdigest()}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
