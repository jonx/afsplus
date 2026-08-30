#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""Build a retained MacAROS FAT, AROS-handler and AFS+ RAM image."""

from __future__ import annotations

import argparse
import hashlib
from pathlib import Path
import struct
import sys


SECTOR_SIZE = 512
ALIGNMENT = 4096
HEADER_SIZE = 4096
MAGIC = b"AFSPRAM\0"
VERSION = 2
FLAG_HANDLER = 1
MAX_PAYLOAD_SIZE = (1 << 32) - 1


class ImageError(ValueError):
    pass


def align_up(value: int, alignment: int) -> int:
    return (value + alignment - 1) & ~(alignment - 1)


def fat_size(image: bytes) -> int:
    if (len(image) < SECTOR_SIZE or image[510:512] != b"\x55\xaa"):
        raise ImageError("system image has no valid FAT boot sector")
    sector_size = struct.unpack_from("<H", image, 11)[0]
    total16 = struct.unpack_from("<H", image, 19)[0]
    total32 = struct.unpack_from("<I", image, 32)[0]
    sectors = total16 or total32
    if sector_size != SECTOR_SIZE or sectors == 0:
        raise ImageError("system image has unsupported FAT geometry")
    size = sectors * SECTOR_SIZE
    if size != len(image):
        raise ImageError(
            f"system image length {len(image)} does not equal FAT size {size}"
        )
    return size


def build(system: bytes, handler: bytes, payload: bytes) -> tuple[bytes, int, int]:
    system_size = fat_size(system)
    if not handler or len(handler) > MAX_PAYLOAD_SIZE:
        raise ImageError("AROS handler size must fit 1..4294967295 bytes")
    if not payload or len(payload) % ALIGNMENT:
        raise ImageError("AFS+ payload size must be a nonzero multiple of 4096")
    if len(payload) > MAX_PAYLOAD_SIZE:
        raise ImageError("AFS+ payload exceeds the 32-bit Alpha-0 device limit")
    header_offset = align_up(system_size, ALIGNMENT)
    handler_offset = header_offset + HEADER_SIZE
    payload_offset = align_up(handler_offset + len(handler), ALIGNMENT)
    header = bytearray(HEADER_SIZE)
    struct.pack_into(
        "<8sIIQIIQQ", header, 0, MAGIC, VERSION, HEADER_SIZE,
        len(payload), SECTOR_SIZE, FLAG_HANDLER, handler_offset, len(handler)
    )
    output = bytearray(payload_offset + len(payload))
    output[: len(system)] = system
    output[header_offset:header_offset + HEADER_SIZE] = header
    output[handler_offset:handler_offset + len(handler)] = handler
    output[payload_offset:] = payload
    return bytes(output), handler_offset, payload_offset


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--system-image", required=True, type=Path)
    parser.add_argument("--aros-handler", required=True, type=Path)
    parser.add_argument("--afsplus-image", required=True, type=Path)
    parser.add_argument("--verify", action="store_true")
    parser.add_argument("output", type=Path)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        expected, handler_offset, payload_offset = build(
            args.system_image.read_bytes(), args.aros_handler.read_bytes(),
            args.afsplus_image.read_bytes()
        )
        if args.verify:
            if args.output.read_bytes() != expected:
                raise ImageError("composite image content mismatch")
        else:
            if args.output.exists():
                raise ImageError(f"refusing to replace {args.output}")
            args.output.write_bytes(expected)
    except (ImageError, OSError) as error:
        print(f"macaros-afsram-image result=FAIL reason={error}", file=sys.stderr)
        return 1
    print(
        "macaros-afsram-image result=PASS "
        f"path={args.output} bytes={len(expected)} "
        f"handler_offset={handler_offset} payload_offset={payload_offset} "
        f"payload_bytes={len(expected) - payload_offset} "
        f"sha256={hashlib.sha256(expected).hexdigest()}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
