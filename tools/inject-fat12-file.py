#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""Inject one deterministic 8.3 file into a FAT12 root or subdirectory."""

from __future__ import annotations

import argparse
import hashlib
from pathlib import Path, PurePosixPath
import struct
import sys


class FatError(ValueError):
    pass


class Fat12:
    def __init__(self, data: bytes):
        self.data = bytearray(data)
        if len(data) < 512 or data[510:512] != b"\x55\xaa":
            raise FatError("invalid FAT boot sector")
        self.sector_size = struct.unpack_from("<H", data, 11)[0]
        self.sectors_per_cluster = data[13]
        self.reserved_sectors = struct.unpack_from("<H", data, 14)[0]
        self.fat_count = data[16]
        self.root_entries = struct.unpack_from("<H", data, 17)[0]
        total16 = struct.unpack_from("<H", data, 19)[0]
        self.total_sectors = total16 or struct.unpack_from("<I", data, 32)[0]
        self.sectors_per_fat = struct.unpack_from("<H", data, 22)[0]
        if (
            self.sector_size != 512
            or self.sectors_per_cluster == 0
            or self.reserved_sectors == 0
            or self.fat_count == 0
            or self.root_entries == 0
            or self.total_sectors == 0
            or self.sectors_per_fat == 0
            or self.total_sectors * self.sector_size != len(data)
        ):
            raise FatError("unsupported FAT12 geometry")
        self.root_sectors = (
            self.root_entries * 32 + self.sector_size - 1
        ) // self.sector_size
        self.root_sector = (
            self.reserved_sectors + self.fat_count * self.sectors_per_fat
        )
        self.data_sector = self.root_sector + self.root_sectors
        data_sectors = self.total_sectors - self.data_sector
        self.cluster_count = data_sectors // self.sectors_per_cluster
        if self.cluster_count == 0 or self.cluster_count >= 4085:
            raise FatError("image is not FAT12")
        self.cluster_size = self.sectors_per_cluster * self.sector_size
        self.fat_size = self.sectors_per_fat * self.sector_size

    def fat_offset(self, cluster: int) -> int:
        if cluster < 2 or cluster >= self.cluster_count + 2:
            raise FatError("cluster outside FAT12 range")
        return cluster + cluster // 2

    def fat_value(self, cluster: int) -> int:
        offset = self.reserved_sectors * self.sector_size + self.fat_offset(cluster)
        value = self.data[offset] | (self.data[offset + 1] << 8)
        return (value >> 4) & 0xFFF if cluster & 1 else value & 0xFFF

    def set_fat_value(self, cluster: int, value: int) -> None:
        if value < 0 or value > 0xFFF:
            raise FatError("FAT12 value outside range")
        relative = self.fat_offset(cluster)
        for index in range(self.fat_count):
            offset = (
                (self.reserved_sectors + index * self.sectors_per_fat)
                * self.sector_size
                + relative
            )
            old = self.data[offset] | (self.data[offset + 1] << 8)
            if cluster & 1:
                new = (old & 0x000F) | (value << 4)
            else:
                new = (old & 0xF000) | value
            self.data[offset] = new & 0xFF
            self.data[offset + 1] = (new >> 8) & 0xFF

    def cluster_offset(self, cluster: int) -> int:
        self.fat_offset(cluster)
        sector = self.data_sector + (cluster - 2) * self.sectors_per_cluster
        return sector * self.sector_size

    def allocate(self, count: int) -> list[int]:
        free = [
            cluster
            for cluster in range(2, self.cluster_count + 2)
            if self.fat_value(cluster) == 0
        ]
        if count <= 0 or len(free) < count:
            raise FatError("not enough free FAT12 clusters")
        selected = free[:count]
        for index, cluster in enumerate(selected):
            following = selected[index + 1] if index + 1 < count else 0xFFF
            self.set_fat_value(cluster, following)
            start = self.cluster_offset(cluster)
            self.data[start:start + self.cluster_size] = bytes(self.cluster_size)
        return selected

    @staticmethod
    def short_name(component: str) -> bytes:
        if component in (".", ".."):
            return component.encode("ascii") + b" " * (11 - len(component))
        if component.count(".") > 1:
            raise FatError(f"not an 8.3 name: {component}")
        stem, dot, suffix = component.partition(".")
        try:
            stem_bytes = stem.upper().encode("ascii")
            suffix_bytes = suffix.upper().encode("ascii")
        except UnicodeEncodeError as error:
            raise FatError(f"not an ASCII 8.3 name: {component}") from error
        allowed = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_$~!#%&-{}()@'`"
        if (
            not stem_bytes
            or len(stem_bytes) > 8
            or len(suffix_bytes) > 3
            or any(value not in allowed for value in stem_bytes + suffix_bytes)
            or (dot and not suffix_bytes)
        ):
            raise FatError(f"not an 8.3 name: {component}")
        return stem_bytes.ljust(8, b" ") + suffix_bytes.ljust(3, b" ")

    @staticmethod
    def entry(name: bytes, attributes: int, cluster: int, size: int) -> bytes:
        if len(name) != 11 or cluster > 0xFFFF or size > 0xFFFFFFFF:
            raise FatError("directory entry value outside FAT12 range")
        result = bytearray(32)
        result[:11] = name
        result[11] = attributes
        struct.pack_into("<H", result, 26, cluster)
        struct.pack_into("<I", result, 28, size)
        return bytes(result)

    @staticmethod
    def lfn_checksum(short_name: bytes) -> int:
        checksum = 0
        for value in short_name:
            checksum = ((checksum & 1) << 7) + (checksum >> 1) + value
            checksum &= 0xFF
        return checksum

    @classmethod
    def lfn_entry(cls, long_name: str, short_name: bytes) -> bytes:
        codepoints = [ord(character) for character in long_name]
        if (
            not codepoints
            or len(codepoints) > 13
            or any(value == 0 or value > 0xFFFF for value in codepoints)
        ):
            raise FatError("long fixture name must fit one UTF-16 LFN entry")
        codepoints.append(0)
        codepoints.extend([0xFFFF] * (13 - len(codepoints)))
        result = bytearray(32)
        result[0] = 0x41
        result[11] = 0x0F
        result[13] = cls.lfn_checksum(short_name)
        offsets = (1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30)
        for offset, value in zip(offsets, codepoints):
            struct.pack_into("<H", result, offset, value)
        return bytes(result)

    @staticmethod
    def find_entry(data: bytearray, start: int, count: int, name: bytes) -> int | None:
        for index in range(count):
            offset = start + index * 32
            first = data[offset]
            if first == 0:
                return None
            if first != 0xE5 and data[offset:offset + 11] == name:
                return offset
        return None

    @staticmethod
    def free_entry(data: bytearray, start: int, count: int) -> int:
        for index in range(count):
            offset = start + index * 32
            if data[offset] in (0, 0xE5):
                return offset
        raise FatError("directory has no free entry")

    @staticmethod
    def free_entries(data: bytearray, start: int, count: int, needed: int) -> int:
        run = 0
        run_start = 0
        for index in range(count):
            offset = start + index * 32
            if data[offset] in (0, 0xE5):
                if run == 0:
                    run_start = offset
                run += 1
                if run == needed:
                    return run_start
            else:
                run = 0
        raise FatError("directory has no contiguous free entries")

    @classmethod
    def file_names(cls, component: str) -> tuple[bytes, str | None]:
        try:
            return cls.short_name(component), None
        except FatError:
            stem, _, suffix = component.partition(".")
            if (
                not stem
                or not suffix
                or component.count(".") != 1
                or len(component) > 13
            ):
                raise
            try:
                stem_ascii = stem.upper().encode("ascii")
                suffix_ascii = suffix.upper().encode("ascii")
            except UnicodeEncodeError as error:
                raise FatError(
                    f"not an ASCII single-entry LFN: {component}"
                ) from error
            allowed = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_$~!#%&-{}()@'`"
            if any(value not in allowed for value in stem_ascii + suffix_ascii):
                raise
            alias = stem_ascii[:6].ljust(6, b"_") + b"~1" + suffix_ascii[:3].ljust(3, b"_")
            return alias, component

    def inject(self, path: PurePosixPath, content: bytes) -> None:
        parts = path.parts
        if not content or len(content) > 0xFFFFFFFF:
            raise FatError("file must contain 1..4294967295 bytes")
        if len(parts) not in (1, 2) or any(part in ("", ".", "..") for part in parts):
            raise FatError("path must be FILE or DIRECTORY/FILE")
        root_start = self.root_sector * self.sector_size
        directory_start = root_start
        directory_entries = self.root_entries
        if len(parts) == 2:
            directory_name = self.short_name(parts[0])
            directory_entry = self.find_entry(
                self.data, root_start, self.root_entries, directory_name
            )
            if directory_entry is None:
                cluster = self.allocate(1)[0]
                directory_entry = self.free_entry(
                    self.data, root_start, self.root_entries
                )
                self.data[directory_entry:directory_entry + 32] = self.entry(
                    directory_name, 0x10, cluster, 0
                )
                directory_start = self.cluster_offset(cluster)
                self.data[directory_start:directory_start + 32] = self.entry(
                    self.short_name("."), 0x10, cluster, 0
                )
                self.data[directory_start + 32:directory_start + 64] = self.entry(
                    self.short_name(".."), 0x10, 0, 0
                )
            else:
                if self.data[directory_entry + 11] & 0x10 == 0:
                    raise FatError(f"{parts[0]} exists but is not a directory")
                cluster = struct.unpack_from("<H", self.data, directory_entry + 26)[0]
                if cluster < 2 or self.fat_value(cluster) < 0xFF8:
                    raise FatError("only one-cluster fixture directories are supported")
                directory_start = self.cluster_offset(cluster)
            directory_entries = self.cluster_size // 32
        file_name, long_name = self.file_names(parts[-1])
        if self.find_entry(
            self.data, directory_start, directory_entries, file_name
        ) is not None:
            raise FatError(f"{path} already exists")
        clusters = self.allocate(
            (len(content) + self.cluster_size - 1) // self.cluster_size
        )
        for index, cluster in enumerate(clusters):
            source = index * self.cluster_size
            chunk = content[source:source + self.cluster_size]
            target = self.cluster_offset(cluster)
            self.data[target:target + len(chunk)] = chunk
        needed_entries = 2 if long_name is not None else 1
        entry_offset = self.free_entries(
            self.data, directory_start, directory_entries, needed_entries
        )
        if long_name is not None:
            self.data[entry_offset:entry_offset + 32] = self.lfn_entry(
                long_name, file_name
            )
            entry_offset += 32
        self.data[entry_offset:entry_offset + 32] = self.entry(
            file_name, 0x20, clusters[0], len(content)
        )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--file", required=True, type=Path)
    parser.add_argument("--path", required=True, type=PurePosixPath)
    parser.add_argument("--verify", action="store_true")
    parser.add_argument("source", type=Path)
    parser.add_argument("output", type=Path)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        fat = Fat12(args.source.read_bytes())
        content = args.file.read_bytes()
        fat.inject(args.path, content)
        expected = bytes(fat.data)
        if args.verify:
            if args.output.read_bytes() != expected:
                raise FatError("injected FAT12 image content mismatch")
        else:
            if args.output.exists():
                raise FatError(f"refusing to replace {args.output}")
            args.output.write_bytes(expected)
    except (FatError, OSError) as error:
        print(f"fat12-inject result=FAIL reason={error}", file=sys.stderr)
        return 1
    print(
        "fat12-inject result=PASS "
        f"path={args.path} bytes={len(content)} output={args.output} "
        f"sha256={hashlib.sha256(expected).hexdigest()}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
