#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""Fresh-process semantic bundle and independently decoded trace regressions."""
import importlib.util
import json
import io
import struct
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("afsptest", Path(__file__).with_name("afsptest.py"))
tool = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tool)
BINARY = tool.ROOT / "target/debug/afsplus-scenario"


def fixture():
    return {"version": 1, "volume": {"block_size": 4096, "blocks": 256, "region_size": 64, "log_slots": 8},
        "operations": [{"op": "create", "label": "f", "parent": "root", "name": "café", "data": "00ff"},
            {"op": "remount"}, {"op": "write", "label": "f", "offset": 1, "data": "42"},
            {"op": "sync"}, {"op": "remount"}],
        "expected": [{"path": ["café"], "kind": "file", "data": "0042"}]}


class ReplayTests(unittest.TestCase):
    def cli(self, *args):
        return subprocess.run([sys.executable, str(Path(tool.__file__)), "--runner", str(BINARY), *map(str, args)],
            capture_output=True, timeout=120)

    def test_fresh_process_success_failure_and_original_immutability(self):
        with tempfile.TemporaryDirectory(prefix="afsplus-replay-") as temporary:
            root = Path(temporary)
            for wrong in (False, True):
                value = fixture()
                if wrong:
                    value["expected"][0]["data"] = "ffff"
                source = root / ("wrong.json" if wrong else "right.json")
                source.write_bytes(tool.encoded(value))
                output = root / ("failure" if wrong else "success")
                run = self.cli("run", source, output)
                self.assertEqual(run.returncode, 2 if wrong else 0, run.stderr)
                before = {p.name: p.read_bytes() for p in output.iterdir()}
                repeat = self.cli("replay", output)
                self.assertEqual(repeat.returncode, run.returncode, repeat.stderr)
                self.assertEqual(before, {p.name: p.read_bytes() for p in output.iterdir()})
                records = tool.bundle.read_bundle(output)
                tool.validate_trace(records)
                self.assertEqual(json.loads(records["run.json"])["outcome"], "failure" if wrong else "pass")

    def test_resealed_source_fault_and_base_mismatch_refuse(self):
        records, success = tool.execute(tool.encoded(fixture()), BINARY)
        self.assertTrue(success)
        with tempfile.TemporaryDirectory(prefix="afsplus-replay-invalid-") as temporary:
            for fault in ("source", "fault", "base", "flight", "trace"):
                edited = dict(records)
                if fault == "source":
                    meta = json.loads(edited["run.json"])
                    meta["source_observed"]["revision"] = "0" * 40
                    edited["run.json"] = tool.encoded(meta)
                elif fault == "fault":
                    edited["fault-model.json"] = tool.encoded({"version": 1, "kind": "arbitrary"})
                else:
                    role = {"base": "start.img", "flight": "flight-recorder.bin", "trace": "block-io.afstrace"}[fault]
                    data = bytearray(edited[role])
                    data[-1] ^= 1
                    edited[role] = bytes(data)
                path = Path(temporary) / fault
                tool.bundle.publish(path, edited)
                with self.assertRaises(ValueError):
                    tool.replay(path, BINARY)

    def test_operation_failure_is_preserved_as_failure_bundle(self):
        value = fixture()
        value["operations"].insert(1, {"op": "create", "label": "duplicate", "parent": "root", "name": "café", "data": ""})
        records, success = tool.execute(tool.encoded(value), BINARY)
        self.assertFalse(success)
        actual = json.loads(records["actual.json"])
        self.assertEqual(actual["failure"]["operation"], 1)
        self.assertEqual(actual["entries"][0]["data"], "00ff")
        tool.validate_trace(records)

    def test_minimizer_preserves_failure_and_original_with_explicit_budget(self):
        value = fixture()
        value["operations"][:0] = [
            {"op": "create", "label": "spare", "parent": "root", "name": "temp", "data": ""},
            {"op": "unlink", "label": "spare"}]
        value["expected"][0]["data"] = "ffff"
        records, success = tool.execute(tool.encoded(value), BINARY)
        self.assertFalse(success)
        with tempfile.TemporaryDirectory(prefix="afsplus-minimize-") as temporary:
            root = Path(temporary)
            original = root / "original"
            tool.bundle.publish(original, records)
            before = {p.name: p.read_bytes() for p in original.iterdir()}
            report = tool.minimize(original, root / "reduced", BINARY)
            self.assertFalse(report["budget_exhausted"])
            self.assertLess(report["operations"], len(value["operations"]))
            reduced = tool.bundle.read_bundle(root / "reduced")
            self.assertEqual(tool.failure_signature(reduced), tool.failure_signature(records))
            self.assertFalse(tool.replay(root / "reduced", BINARY))
            self.assertEqual(before, {p.name: p.read_bytes() for p in original.iterdir()})
            limited = tool.minimize(original, root / "limited", BINARY, max_runs=1)
            self.assertTrue(limited["budget_exhausted"])
            self.assertFalse(tool.replay(root / "limited", BINARY))

    def test_framed_output_rejects_truncation_roles_and_sizes(self):
        wire = bytearray(b"AFSRUN01")
        for role in tool.RUNNER_ROLES:
            raw = role.encode()
            wire.extend(struct.pack("<H", len(raw)) + raw + struct.pack("<Q", 0))
        self.assertEqual(set(tool.unframe(io.BytesIO(wire), 0, 0)), set(tool.RUNNER_ROLES))
        for stop in range(len(wire)):
            with self.assertRaises(ValueError):
                tool.unframe(io.BytesIO(wire[:stop]), 0, 0)
        for bad in (b"BADRUN01" + wire[8:], wire + b"x", wire[:10] + b"x" + wire[11:]):
            with self.assertRaises(ValueError):
                tool.unframe(io.BytesIO(bad), 0, 0)

    def test_export_budget_refuses_before_runner(self):
        with self.assertRaisesRegex(ValueError, "per-file budget"):
            tool.execute(tool.encoded(fixture()), Path("/no/such/runner"), file_bytes=4096)
        with self.assertRaisesRegex(ValueError, "aggregate budget"):
            tool.execute(tool.encoded(fixture()), Path("/no/such/runner"), total_bytes=4096)


if __name__ == "__main__":
    unittest.main()
