#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""Independently assemble ADR-073 golden blocks; default checks without writes."""
import argparse
import json
from pathlib import Path
import struct


def images():
    outputs = {}
    for name, extended in [("checkpoint-legacy-v1", False), ("checkpoint-snapshot-v1", True)]:
        block = bytearray(4096)
        block[:4] = b"AFSC"
        struct.pack_into("<H", block, 4, 1)
        struct.pack_into("<Q", block, 16, 5)
        struct.pack_into("<I", block, 24, 112 if extended else 96)
        block[32:48] = bytes([7]) * 16
        for offset, value in [(16, 5), (24, 1), (32, 10), (40, 12), (48, 11), (56, 20), (64, 5), (72, 800)]:
            struct.pack_into("<Q", block, 32 + offset, value)
        if extended:
            struct.pack_into("<QQ", block, 128, 30, 31)
        crc = 0xFFFFFFFF
        for byte in block:
            crc ^= byte
            for _ in range(8):
                crc = (crc >> 1) ^ (0x82F63B78 if crc & 1 else 0)
        struct.pack_into("<I", block, 28, crc ^ 0xFFFFFFFF)
        outputs[name + ".bin"] = bytes(block)
    manifest = {
        "schema_version": 1,
        "scope": "standalone checkpoint blocks",
        "generation": 5,
        "uuid": "07" * 16,
        "images": [
            {"file": "checkpoint-legacy-v1.bin", "payload_bytes": 96, "snapshot_roots": None},
            {"file": "checkpoint-snapshot-v1.bin", "payload_bytes": 112, "snapshot_roots": {"registry": 30, "lifetimes": 31}},
        ],
    }
    outputs["checkpoint-roots.json"] = (json.dumps(manifest, indent=2) + "\n").encode()
    return outputs


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--write", action="store_true", help="regenerate the three fixed fixtures")
    args = parser.parse_args()
    directory = Path(__file__).resolve().parent
    failures = []
    for name, expected in images().items():
        path = directory / name
        if args.write:
            path.write_bytes(expected)
        elif not path.exists() or path.read_bytes() != expected:
            failures.append(name)
    if failures:
        parser.exit(1, "checkpoint fixtures differ: " + ", ".join(failures) + "\n")
    print("checkpoint-fixtures result=PASS mode=" + ("write" if args.write else "check"))


if __name__ == "__main__":
    main()
