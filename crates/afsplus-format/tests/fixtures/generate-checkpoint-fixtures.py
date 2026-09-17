#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""Independently assemble the golden checkpoint blocks; default checks without writes.

The v2 images carry the 72-byte label field of ADR-104 at payload offset 96
(length, seven reserved bytes, 64 bytes of NUL-free UTF-8, zero padded) and,
when present, the ADR-073 snapshot roots after it. The v1 images are the
retired layout without a label field; every reader refuses them.
"""
import argparse
import json
from pathlib import Path
import struct


def images():
    outputs = {}
    label = "Test Volume".encode()
    for name, extended, labelled in [
        ("checkpoint-legacy-v1", False, False),
        ("checkpoint-snapshot-v1", True, False),
        ("checkpoint-label-v2", False, True),
        ("checkpoint-snapshot-v2", True, True),
    ]:
        roots_offset = 168 if labelled else 96
        block = bytearray(4096)
        block[:4] = b"AFSC"
        struct.pack_into("<H", block, 4, 1)
        struct.pack_into("<Q", block, 16, 5)
        struct.pack_into("<I", block, 24, roots_offset + (16 if extended else 0))
        block[32:48] = bytes([7]) * 16
        for offset, value in [(16, 5), (24, 1), (32, 10), (40, 12), (48, 11), (56, 20), (64, 5), (72, 800)]:
            struct.pack_into("<Q", block, 32 + offset, value)
        if labelled:
            block[32 + 96] = len(label)
            block[32 + 104:32 + 104 + len(label)] = label
        if extended:
            struct.pack_into("<QQ", block, 32 + roots_offset, 30, 31)
        crc = 0xFFFFFFFF
        for byte in block:
            crc ^= byte
            for _ in range(8):
                crc = (crc >> 1) ^ (0x82F63B78 if crc & 1 else 0)
        struct.pack_into("<I", block, 28, crc ^ 0xFFFFFFFF)
        outputs[name + ".bin"] = bytes(block)
    manifest = {
        "schema_version": 2,
        "scope": "standalone checkpoint blocks",
        "generation": 5,
        "uuid": "07" * 16,
        "images": [
            {"file": "checkpoint-label-v2.bin", "payload_bytes": 168, "label": "Test Volume", "snapshot_roots": None},
            {"file": "checkpoint-snapshot-v2.bin", "payload_bytes": 184, "label": "Test Volume", "snapshot_roots": {"registry": 30, "lifetimes": 31}},
            {"file": "checkpoint-legacy-v1.bin", "payload_bytes": 96, "retired": True, "snapshot_roots": None},
            {"file": "checkpoint-snapshot-v1.bin", "payload_bytes": 112, "retired": True, "snapshot_roots": {"registry": 30, "lifetimes": 31}},
        ],
    }
    outputs["checkpoint-roots.json"] = (json.dumps(manifest, indent=2) + "\n").encode()
    return outputs


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--write", action="store_true", help="regenerate the fixed fixtures")
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
