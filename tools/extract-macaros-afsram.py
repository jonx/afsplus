#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""Extract the unique AFS+ payload from file-backed MacAROS guest RAM."""

from __future__ import annotations

import argparse
import hashlib
import mmap
from pathlib import Path
import struct
import sys


MAGIC = b"AFSPRAM\0"
AFS_MAGIC = b"AFSI"
VERSION = 2
HEADER_SIZE = 4096
ALIGNMENT = 4096
SECTOR_SIZE = 512
FLAG_HANDLER = 1
HEADER = struct.Struct("<8sIIQIIQQ")
COPY_CHUNK = 1024 * 1024


class ExtractError(ValueError):
    pass


def align_up(value: int) -> int:
    return (value + ALIGNMENT - 1) & ~(ALIGNMENT - 1)


def locate(memory: mmap.mmap) -> tuple[int, int, int]:
    matches: list[tuple[int, int, int]] = []
    cursor = 0
    while True:
        descriptor = memory.find(MAGIC, cursor)
        if descriptor < 0:
            break
        cursor = descriptor + 1
        if descriptor % ALIGNMENT or descriptor + HEADER_SIZE > len(memory):
            continue
        try:
            (
                magic,
                version,
                header_size,
                payload_size,
                sector_size,
                flags,
                handler_offset,
                handler_size,
            ) = HEADER.unpack_from(memory, descriptor)
        except struct.error:
            continue
        if (
            magic != MAGIC
            or version != VERSION
            or header_size != HEADER_SIZE
            or sector_size != SECTOR_SIZE
            or flags != FLAG_HANDLER
            or payload_size == 0
            or payload_size % ALIGNMENT
            or handler_offset < HEADER_SIZE
            or handler_size == 0
            or any(memory[descriptor + HEADER.size:descriptor + HEADER_SIZE])
        ):
            continue
        image_start = descriptor + HEADER_SIZE - handler_offset
        handler_end = handler_offset + handler_size
        payload_relative = align_up(handler_end)
        payload_start = image_start + payload_relative
        payload_end = payload_start + payload_size
        if (
            image_start < 0
            or image_start + 512 > descriptor
            or payload_start < descriptor + HEADER_SIZE
            or payload_end > len(memory)
            or memory[image_start + 510:image_start + 512] != b"\x55\xaa"
            or struct.unpack_from("<H", memory, image_start + 11)[0]
            != SECTOR_SIZE
        ):
            continue
        sectors16 = struct.unpack_from("<H", memory, image_start + 19)[0]
        sectors = sectors16 or struct.unpack_from(
            "<I", memory, image_start + 32
        )[0]
        if (
            sectors == 0
            or align_up(sectors * SECTOR_SIZE) != descriptor - image_start
            or any(
                memory[
                    image_start + handler_end:image_start + payload_relative
                ]
            )
            or memory[payload_start:payload_start + len(AFS_MAGIC)] != AFS_MAGIC
        ):
            continue
        matches.append((image_start, payload_start, payload_size))
    if len(matches) != 1:
        raise ExtractError(
            f"expected one strict retained-image descriptor, found {len(matches)}"
        )
    return matches[0]


def compare(memory: mmap.mmap, start: int, size: int, output: Path) -> None:
    try:
        with output.open("rb") as stream:
            position = 0
            while position < size:
                count = min(COPY_CHUNK, size - position)
                if stream.read(count) != memory[start + position:start + position + count]:
                    raise ExtractError("extracted AFS+ payload content mismatch")
                position += count
            if stream.read(1):
                raise ExtractError("extracted AFS+ payload has trailing bytes")
    except OSError as error:
        raise ExtractError(f"cannot verify {output}: {error}") from error


def write(memory: mmap.mmap, start: int, size: int, output: Path) -> None:
    if output.exists():
        raise ExtractError(f"refusing to replace {output}")
    try:
        with output.open("xb") as stream:
            position = 0
            while position < size:
                count = min(COPY_CHUNK, size - position)
                stream.write(memory[start + position:start + position + count])
                position += count
    except OSError as error:
        if output.exists():
            output.unlink()
        raise ExtractError(f"cannot write {output}: {error}") from error


def digest(memory: mmap.mmap, start: int, size: int) -> str:
    result = hashlib.sha256()
    position = 0
    while position < size:
        count = min(COPY_CHUNK, size - position)
        result.update(memory[start + position:start + position + count])
        position += count
    return result.hexdigest()


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--verify", action="store_true")
    parser.add_argument("guest_memory", type=Path)
    parser.add_argument("output", type=Path)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        with args.guest_memory.open("rb") as stream:
            with mmap.mmap(stream.fileno(), 0, access=mmap.ACCESS_READ) as memory:
                image_start, payload_start, payload_size = locate(memory)
                if args.verify:
                    compare(memory, payload_start, payload_size, args.output)
                else:
                    write(memory, payload_start, payload_size, args.output)
                sha256 = digest(memory, payload_start, payload_size)
    except (ExtractError, OSError, ValueError) as error:
        print(f"macaros-afsram-extract result=FAIL reason={error}", file=sys.stderr)
        return 1
    print(
        "macaros-afsram-extract result=PASS "
        f"memory={args.guest_memory} image_offset={image_start} "
        f"payload_offset={payload_start} payload_bytes={payload_size} "
        f"output={args.output} sha256={sha256}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
