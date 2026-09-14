#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""Compare a feature-built origin meter with the ordinary host workload."""
import argparse
import json
from pathlib import Path
import subprocess
import unittest

ROOT = Path(__file__).resolve().parent.parent
TAGGED = None
BASELINE = ROOT / "target/debug/afsplus-measure"


def run(binary, arguments):
    result = subprocess.run([str(binary), *arguments], capture_output=True, timeout=120)
    if result.returncode:
        raise AssertionError(result.stderr.decode())
    return json.loads(result.stdout)


class OriginTests(unittest.TestCase):
    def test_all_profiles_balance_origins_and_preserve_image_io(self):
        for pages in (None, 2, 4, 8, "unlimited"):
            arguments = [] if pages is None else ["--cache-profile", str(pages)]
            baseline = run(BASELINE, arguments)
            tagged = run(TAGGED, arguments)
            self.assertEqual(baseline["version"], 1)
            self.assertEqual(tagged["version"], 3)
            self.assertEqual(tagged["allocation_profile"], "requested-origins-v1")
            self.assertEqual(tagged["image_crc32c"], baseline["image_crc32c"])
            self.assertEqual(len(tagged["phases"]), len(baseline["phases"]))
            for row, plain in zip(tagged["phases"], baseline["phases"]):
                self.assertEqual(row["name"], plain["name"])
                for field in ("reads", "writes", "bytes_read", "bytes_written", "flushes",
                              "logical_payload_written_bytes", "logical_payload_read_bytes"):
                    self.assertEqual(row[field], plain[field])
                self.check_row(row)
                self.assertEqual(row["allocation_origins"]["fixture"]["start_bytes"], tagged["image_bytes"])
                self.assertEqual(row["allocation_origins"]["fixture"]["end_bytes"], tagged["image_bytes"])
            mutation = next(r for r in tagged["phases"] if r["name"] in ("create", "batch-create"))
            for origin in ("allocator", "tree"):
                self.assertGreater(mutation["allocation_origins"][origin]["acquired_bytes"], 0)
            if pages is not None:
                self.assertGreater(mutation["allocation_origins"]["batch"]["acquired_bytes"], 0)
            final = tagged["phases"][-1]
            self.assertEqual(tagged["phases"][0]["heap_start_bytes"], final["heap_end_bytes"])
            for origin in ("tree", "batch", "allocator", "verifier", "oracle"):
                self.assertEqual(final["allocation_origins"][origin]["end_bytes"], 0)

    def check_row(self, row):
        origins = row["allocation_origins"]
        self.assertEqual(set(origins), {"other", "fixture", "reporting", "allocator", "tree",
                                       "batch", "snapshot", "verifier", "oracle"})
        for field, global_field in (("start_bytes", "heap_start_bytes"), ("end_bytes", "heap_end_bytes"),
                ("acquired_bytes", "heap_acquired_bytes"), ("released_bytes", "heap_released_bytes")):
            self.assertEqual(sum(origin[field] for origin in origins.values()), row[global_field])
        # Independent peaks do not form a simultaneous global peak.
        self.assertGreaterEqual(sum(o["peak_bytes"] for o in origins.values()), row["heap_peak_bytes"])
        for sample in [*origins.values(), row["tracking_overhead"], row["underlying_requests"]]:
            self.assertEqual(sample["end_bytes"]-sample["start_bytes"], sample["acquired_bytes"]-sample["released_bytes"])
            self.assertGreaterEqual(sample["peak_bytes"], max(sample["start_bytes"], sample["end_bytes"]))
        for point in ("start", "end"):
            self.assertEqual(row["underlying_requests"][point+"_bytes"],
                row["heap_"+point+"_bytes"]+row["tracking_overhead"][point+"_bytes"])
        self.assertGreater(row["tracking_overhead"]["peak_bytes"], 0)

    def test_oracle_origin_and_resident_mode_compose(self):
        for pages in (2, "unlimited"):
            report = run(TAGGED, ["--cache-profile", str(pages), "--resident-rounds", "3"])
            self.assertEqual(report["version"], 3)
            self.assertEqual(report["steady_read"]["rounds"], 3)
            rows = report["phases"]
            for row in rows:
                self.check_row(row)
                self.assertGreater(row["resident_end_bytes"], 0)
            steady = [r for r in rows if r["name"] == "steady-read"]
            self.assertEqual(len(steady), 3)
            for row in steady:
                oracle = row["allocation_origins"]["oracle"]
                self.assertGreater(oracle["start_bytes"], row["logical_payload_read_bytes"])
                self.assertEqual(oracle["start_bytes"], oracle["end_bytes"])
            release = next(r for r in rows if r["name"] == "steady-release")
            self.assertEqual(release["allocation_origins"]["oracle"]["end_bytes"], 0)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tagged", required=True, type=Path)
    parser.add_argument("--baseline", default=BASELINE, type=Path)
    args, remaining = parser.parse_known_args()
    TAGGED, BASELINE = args.tagged.resolve(), args.baseline.resolve()
    unittest.main(argv=[__file__, *remaining])
