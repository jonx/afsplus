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

    def test_cache_profiles_measure_actual_spills_and_balanced_heap(self):
        for pages in (2, 4, 8, "unlimited"):
            result = subprocess.run([BINARY, "--cache-profile", str(pages)],
                                    capture_output=True, check=True, timeout=60)
            report = json.loads(result.stdout)
            self.assertEqual(report["workload"], "tree-cache-batch-v1")
            self.assertEqual(report["outcome"], "pass")
            self.assertEqual(report["cache_pages"], pages)
            trees = report["tree_phases"]
            self.assertEqual([t["phase"] for t in trees], ["batch-create", "batch-delete"])
            if pages == "unlimited":
                self.assertTrue(all(t["spill_writes"] == 0 for t in trees))
            else:
                self.assertGreater(trees[0]["spill_writes"], 0)
                self.assertTrue(all(t["staged_peak_pages"] <= pages for t in trees))
                self.assertTrue(all(t["staged_before_eviction_peak_pages"] <= pages + 1 for t in trees))
            rows = report["phases"]
            self.assertEqual(rows[0]["heap_start_bytes"], rows[-1]["heap_end_bytes"])
            for row in rows:
                self.assertEqual(row["heap_end_bytes"] - row["heap_start_bytes"],
                                 row["heap_acquired_bytes"] - row["heap_released_bytes"])
                self.assertGreaterEqual(row["heap_peak_bytes"], max(
                    row["heap_start_bytes"], row["heap_end_bytes"]))
                self.assertEqual(row["bytes_written"], row["writes"] * 4096)
                if row["name"].endswith("check"):
                    self.assertEqual(row["writes"], 0)

    def test_resident_series_preserves_workload_and_accounts_for_its_oracle(self):
        for pages in (None, 2, 4, 8, "unlimited"):
            profile = [] if pages is None else ["--cache-profile", str(pages)]
            plain = json.loads(subprocess.run([BINARY, *profile], capture_output=True,
                check=True, timeout=60).stdout)
            measured = json.loads(subprocess.run([BINARY, *profile, "--resident-rounds", "3"],
                capture_output=True, check=True, timeout=60).stdout)
            self.assertEqual(measured["version"], 2)
            self.assertEqual(measured["image_crc32c"], plain["image_crc32c"])
            self.assertEqual(measured["resident_provider"], "ps-rss-kib-v1")
            rows = measured["phases"]
            for row in rows:
                for field in ("resident_start_bytes", "resident_end_bytes"):
                    self.assertGreater(row[field], 0)
                    self.assertEqual(row[field] % 1024, 0)
                self.assertEqual(row["heap_end_bytes"] - row["heap_start_bytes"],
                    row["heap_acquired_bytes"] - row["heap_released_bytes"])
            original = {r["name"]: r for r in plain["phases"]}
            for row in rows:
                if row["name"] not in original:
                    continue
                for field in ("reads", "writes", "flushes", "bytes_read", "bytes_written"):
                    self.assertEqual(row[field], original[row["name"]][field])
            steady = [r for r in rows if r["name"] == "steady-read"]
            self.assertEqual(len(steady), 3)
            expected_bytes = original["read-verify"]["logical_payload_read_bytes"]
            for row in steady:
                self.assertEqual((row["writes"], row["flushes"]), (0,0))
                self.assertEqual(row["logical_payload_read_bytes"], expected_bytes)
                self.assertEqual(row["heap_start_bytes"], row["heap_end_bytes"])
                self.assertEqual(row["reads"], steady[0]["reads"])
            prepare = next(r for r in rows if r["name"] == "steady-prepare")
            release = next(r for r in rows if r["name"] == "steady-release")
            self.assertEqual(prepare["logical_payload_read_bytes"], expected_bytes)
            self.assertEqual(prepare["heap_end_bytes"] - prepare["heap_start_bytes"],
                release["heap_start_bytes"] - release["heap_end_bytes"])
            series = measured["steady_read"]
            values = [r["resident_end_bytes"] for r in steady]
            self.assertEqual((series["end_min_bytes"],series["end_max_bytes"]), (min(values),max(values)))
            self.assertFalse(series["plateau_verified"])
            self.assertEqual(rows[0]["heap_start_bytes"], rows[-1]["heap_end_bytes"])

    def test_resident_admission_rejects_missing_duplicate_and_unbounded_rounds(self):
        for arguments in (["--resident-rounds"], ["--resident-rounds", "0"],
                ["--resident-rounds", "2"], ["--resident-rounds", "33"],
                ["--resident-rounds", "03"], ["--resident-rounds", "3", "--resident-rounds", "3"]):
            result = subprocess.run([BINARY, *arguments], capture_output=True, timeout=5)
            self.assertEqual(result.returncode, 1)
            self.assertEqual(result.stdout, b"")

    def test_invalid_cache_profiles_refuse_without_report(self):
        for pages in ("0", "1", "3", "999", "image.img"):
            result = subprocess.run([BINARY, "--cache-profile", pages],
                                    capture_output=True, timeout=5)
            self.assertEqual(result.returncode, 1)
            self.assertEqual(result.stdout, b"")


if __name__ == "__main__":
    unittest.main()
