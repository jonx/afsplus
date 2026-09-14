#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""Validate the built memory-only heap/I/O workload and its measurement scope."""
import json
from pathlib import Path
import subprocess
import unittest

ROOT = Path(__file__).resolve().parent.parent
BINARY = ROOT / "target/debug/afsplus-measure"


class WorkloadTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.runs = []
        for _ in range(2):
            result = subprocess.run([BINARY], capture_output=True, check=True, timeout=60)
            cls.runs.append(json.loads(result.stdout))

    def test_heap_balance_peak_and_checker_scope(self):
        for report in self.runs:
            self.assertEqual(report["version"], 1)
            self.assertEqual(report["outcome"], "pass")
            rows = report["phases"]
            self.assertEqual([r["name"] for r in rows], [
                "format", "mount", "create", "edit", "sync", "unmount", "raw-check",
                "remount", "read-verify", "final-unmount", "recovered-check",
            ])
            for row in rows:
                self.assertEqual(row["heap_end_bytes"] - row["heap_start_bytes"],
                                 row["heap_acquired_bytes"] - row["heap_released_bytes"])
                self.assertGreaterEqual(row["heap_peak_bytes"], max(
                    row["heap_start_bytes"], row["heap_end_bytes"]))
                self.assertEqual(row["heap_peak_above_start_bytes"],
                                 row["heap_peak_bytes"] - row["heap_start_bytes"])
                self.assertGreaterEqual(row["heap_start_bytes"], report["image_bytes"])
                self.assertEqual(row["bytes_read"], row["reads"] * 4096)
                self.assertEqual(row["bytes_written"], row["writes"] * 4096)
                if row["name"].endswith("check"):
                    self.assertEqual(row["writes"], 0)
                    self.assertEqual(row["flushes"], 0)
                    self.assertGreater(row["heap_peak_above_start_bytes"], 0)
                    self.assertEqual(row["heap_start_bytes"], row["heap_end_bytes"])
            self.assertEqual(rows[0]["heap_start_bytes"], rows[-1]["heap_end_bytes"])

    def test_repeatable_io_and_semantic_denominators(self):
        left, right = self.runs
        self.assertEqual(left["image_crc32c"], right["image_crc32c"])
        fields = ["reads", "writes", "bytes_read", "bytes_written", "flushes",
                  "logical_payload_written_bytes", "logical_payload_read_bytes"]
        for a, b in zip(left["phases"], right["phases"]):
            for field in fields:
                self.assertEqual(a[field], b[field])
        rows = {r["name"]: r for r in left["phases"]}
        self.assertEqual(rows["create"]["logical_payload_written_bytes"], 96000)
        self.assertEqual(rows["edit"]["logical_payload_written_bytes"], 256)
        self.assertEqual(rows["read-verify"]["logical_payload_read_bytes"], 55168)
        self.assertGreater(rows["create"]["bytes_written"], 96000)

    def test_no_host_path_or_other_arguments_accepted(self):
        result = subprocess.run([BINARY, "unexpected.img"], capture_output=True, timeout=5)
        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stdout, b"")
        self.assertIn(b"no arguments", result.stderr)


if __name__ == "__main__":
    unittest.main()
