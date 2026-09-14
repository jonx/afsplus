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
    def test_v3_internal_flight_replays_all_profiles_and_reports_overwrite(self):
        for pages in (2, 4, 8, "unlimited"):
            for capacity in (1, 32):
                value = fixture()
                value.update(version=3, flight_capacity=capacity)
                value["volume"]["tree_cache_pages"] = pages
                records, success = tool.execute(tool.encoded(value), BINARY)
                self.assertTrue(success)
                flight = records["flight-recorder.bin"]
                self.assertEqual(flight[:8], b"AFSFLT02")
                self.assertEqual(struct.unpack_from("<I", flight, 12)[0], capacity)
                lost, count = struct.unpack_from("<QI", flight, 16 + 29)
                self.assertEqual((lost, count), (5, 1) if capacity == 1 else (0, 6))
                tool.validate_trace(records)
                with tempfile.TemporaryDirectory(prefix="afsplus-internal-flight-") as temporary:
                    path = Path(temporary) / "bundle"
                    tool.bundle.publish(path, records)
                    before = {p.name: p.read_bytes() for p in path.iterdir()}
                    result = self.cli("replay", path)
                    self.assertEqual(result.returncode, 0, result.stderr.decode())
                    self.assertEqual(before, {p.name: p.read_bytes() for p in path.iterdir()})

    def test_v3_internal_flight_rejects_resealed_corruption(self):
        value = fixture()
        value.update(version=3, flight_capacity=32)
        value["volume"]["tree_cache_pages"] = 2
        records, success = tool.execute(tool.encoded(value), BINARY)
        self.assertTrue(success)
        # Capacity, loss accounting, count, sequence, attempt, generation,
        # event kind and boolean are independently checked after re-sealing.
        first_event = 16 + 29 + 12
        changes = [(12, "I", 1), (16+29, "Q", 1), (16+29+8, "I", 257),
            (first_event, "Q", 2), (first_event+8, "Q", 0),
            (first_event+16, "Q", 0), (first_event+24, "B", 0),
            (first_event+25, "B", 2)]
        for offset, fmt, number in changes:
            flight = bytearray(records["flight-recorder.bin"])
            struct.pack_into("<"+fmt, flight, offset, number)
            edited = dict(records, **{"flight-recorder.bin": bytes(flight)})
            with self.assertRaisesRegex(ValueError, "flight"):
                tool.validate_trace(edited)
        for wire in (records["flight-recorder.bin"][:-1], records["flight-recorder.bin"]+b"x",
                     b"AFSFLT01"+records["flight-recorder.bin"][8:]):
            with self.assertRaisesRegex(ValueError, "flight"):
                tool.validate_trace(dict(records, **{"flight-recorder.bin": wire}))
        for capacity in (0, 257, True, "32"):
            value["flight_capacity"] = capacity
            with self.assertRaises(ValueError):
                tool.scenario.validate(tool.encoded(value))

    def test_v3_minimization_and_selected_cut_keep_diagnostic_policy(self):
        value = fixture()
        value.update(version=3, flight_capacity=1)
        value["volume"]["tree_cache_pages"] = 2
        value["operations"][:0] = [
            {"op": "create", "label": "spare", "parent": "root", "name": "temp", "data": ""},
            {"op": "unlink", "label": "spare"}]
        value["expected"][0]["data"] = "ffff"
        records, success = tool.execute(tool.encoded(value), BINARY)
        self.assertFalse(success)
        self.assertEqual(json.loads(tool.failure_signature(records))["flight_capacity"], 1)
        with tempfile.TemporaryDirectory(prefix="afsplus-flight-minimize-") as temporary:
            root = Path(temporary)
            tool.bundle.publish(root / "original", records)
            result = tool.minimize(root / "original", root / "reduced", BINARY)
            self.assertLess(result["operations"], len(value["operations"]))
            reduced = tool.bundle.read_bundle(root / "reduced")
            self.assertEqual(tool.failure_signature(reduced), tool.failure_signature(records))
            self.assertEqual(json.loads(reduced["operations.afstrace"])["flight_capacity"], 1)
            self.assertFalse(tool.replay(root / "reduced", BINARY))
        value["operations"] = [{"op": "create", "label": "f", "parent": "root", "name": "a", "data": "01"}]
        value["expected"] = []
        for variant in range(5):
            records, success = tool.execute(tool.encoded(value), BINARY,
                fault={"version": 1, "kind": "power-cut-v1", "operation": 0, "offset": 1, "variant": variant})
            self.assertTrue(success)
            tool.validate_trace(records)
            # The full recording is retained; a selected cut is not a claim
            # that these later commit events occurred on the cut device.
            self.assertEqual(records["flight-recorder.bin"][:8], b"AFSFLT02")

    def test_v2_ladder_profiles_survive_fresh_replay_without_artifact_writes(self):
        for pages in (2, 4, 8, "unlimited"):
            value = fixture()
            value["version"] = 2
            value["volume"]["tree_cache_pages"] = pages
            value["operations"] = [
                {"op": "mkdir", "label": "d", "parent": "root", "name": "src"},
                {"op": "create", "label": "f", "parent": "d", "name": "café", "data": "00ff"},
                {"op": "write", "label": "f", "offset": 4, "data": "42"},
                {"op": "truncate", "label": "f", "size": 2},
                {"op": "rename", "label": "f", "parent": "root", "name": "out"},
                {"op": "rmdir", "label": "d"},
                {"op": "create", "label": "spare", "parent": "root", "name": "temp", "data": ""},
                {"op": "unlink", "label": "spare"}, {"op": "sync"}, {"op": "remount"}]
            value["expected"] = [{"path": ["out"], "kind": "file", "data": "00ff"}]
            records, success = tool.execute(tool.encoded(value), BINARY)
            self.assertTrue(success)
            actual = json.loads(records["actual.json"])
            self.assertEqual(actual["version"], 3)
            self.assertEqual(actual["cache_pages"], pages)
            with tempfile.TemporaryDirectory(prefix="afsplus-profile-replay-") as temporary:
                path = Path(temporary) / "bundle"
                tool.bundle.publish(path, records)
                before = {p.name: p.read_bytes() for p in path.iterdir()}
                result = self.cli("replay", path)
                self.assertEqual(result.returncode, 0, result.stderr.decode())
                self.assertEqual(before, {p.name: p.read_bytes() for p in path.iterdir()})

    def test_v2_profile_binding_rejects_resealed_mismatches_and_silent_fallback(self):
        value = fixture()
        value["version"] = 2
        value["volume"]["tree_cache_pages"] = 2
        records, success = tool.execute(tool.encoded(value), BINARY)
        self.assertTrue(success)
        for change in ({"cache_pages": 4}, {"cache_pages": True}, {"version": 2}, {"cache_pages": "2"}):
            edited = dict(records)
            actual = json.loads(records["actual.json"])
            actual.update(change)
            edited["actual.json"] = tool.encoded(actual)
            with self.assertRaisesRegex(ValueError, "binding"):
                tool.validate_trace(edited)
        edited = dict(records)
        actual = json.loads(records["actual.json"])
        del actual["cache_pages"]
        edited["actual.json"] = tool.encoded(actual)
        with tempfile.TemporaryDirectory(prefix="afsplus-profile-mismatch-") as temporary:
            path = Path(temporary) / "resealed"
            tool.bundle.publish(path, edited)
            with self.assertRaisesRegex(ValueError, "binding"):
                tool.replay(path, BINARY)
        for text in ("02", "0", "true", "Unlimited"):
            with self.assertRaises(ValueError):
                tool.observation(f"AFSOBS03\ncache-pages {text}\nrun ok\nraw-check -\nrecovered-check -\nobserve ok\n".encode())

    def test_v2_minimization_keeps_profile_in_scenario_observation_and_signature(self):
        for pages in (2, 4, 8, "unlimited"):
            value = fixture()
            value["version"] = 2
            value["volume"]["tree_cache_pages"] = pages
            value["operations"][:0] = [
                {"op": "create", "label": "spare", "parent": "root", "name": "temp", "data": ""},
                {"op": "unlink", "label": "spare"}]
            value["expected"][0]["data"] = "ffff"
            records, success = tool.execute(tool.encoded(value), BINARY)
            self.assertFalse(success)
            self.assertEqual(json.loads(tool.failure_signature(records))["tree_cache_pages"], pages)
            other = dict(records)
            changed_value = json.loads(records["operations.afstrace"])
            changed_value["volume"]["tree_cache_pages"] = 4 if pages == 2 else 2
            changed_actual = json.loads(records["actual.json"])
            changed_actual["cache_pages"] = 4 if pages == 2 else 2
            other["operations.afstrace"] = tool.encoded(changed_value)
            other["actual.json"] = tool.encoded(changed_actual)
            self.assertNotEqual(tool.failure_signature(records), tool.failure_signature(other))
            with tempfile.TemporaryDirectory(prefix="afsplus-profile-minimize-") as temporary:
                root = Path(temporary)
                tool.bundle.publish(root / "original", records)
                result = tool.minimize(root / "original", root / "reduced", BINARY)
                self.assertLess(result["operations"], len(value["operations"]))
                reduced = tool.bundle.read_bundle(root / "reduced")
                self.assertEqual(json.loads(reduced["operations.afstrace"])["volume"]["tree_cache_pages"], pages)
                self.assertEqual(json.loads(reduced["actual.json"])["cache_pages"], pages)
                self.assertEqual(tool.failure_signature(reduced), tool.failure_signature(records))
                self.assertFalse(tool.replay(root / "reduced", BINARY))

    def test_v2_selected_crash_observation_uses_each_cache_profile(self):
        for pages in (2, 4, 8, "unlimited"):
            value = fixture()
            value["version"] = 2
            value["volume"]["tree_cache_pages"] = pages
            value["expected"] = []
            fault = {"version": 1, "kind": "power-cut-v1", "operation": 0, "offset": 1, "variant": 0}
            records, success = tool.execute(tool.encoded(value), BINARY, fault=fault)
            self.assertTrue(success)
            self.assertEqual(json.loads(records["actual.json"])["cache_pages"], pages)
            self.assertTrue(tool.verify_replay(records, BINARY, tool.bundle.DEFAULT_FILE_BYTES, tool.bundle.DEFAULT_TOTAL_BYTES))

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

    def test_selected_crashes_replay_and_minimize_with_a_stable_anchor(self):
        value = {"version": 1, "volume": fixture()["volume"],
            "operations": [{"op": "create", "label": "f", "parent": "root", "name": "a", "data": "01"}],
            "expected": []}
        with tempfile.TemporaryDirectory(prefix="afsplus-cut-") as temporary:
            root = Path(temporary)
            for variant in range(5):  # One unflushed write: loss/full plus three tears.
                fault = {"version": 1, "kind": "power-cut-v1", "operation": 0, "offset": 1, "variant": variant}
                records, success = tool.execute(tool.encoded(value), BINARY, fault=fault)
                self.assertTrue(success)
                tool.validate_trace(records)
                output = root / str(variant)
                tool.bundle.publish(output, records)
                result = self.cli("replay", output)
                self.assertEqual(result.returncode, 0, result.stderr)
            value["operations"][:0] = [
                {"op": "create", "label": "spare", "parent": "root", "name": "temp", "data": ""},
                {"op": "unlink", "label": "spare"}]
            value["operations"].append({"op": "sync"})
            value["expected"] = [{"path": ["missing"], "kind": "file", "data": ""}]
            fault = {"version": 1, "kind": "power-cut-v1", "operation": 2, "offset": 1, "variant": 0}
            records, success = tool.execute(tool.encoded(value), BINARY, fault=fault)
            self.assertFalse(success)
            original = root / "original"
            tool.bundle.publish(original, records)
            report = tool.minimize(original, root / "reduced", BINARY)
            self.assertFalse(report["budget_exhausted"])
            reduced = tool.bundle.read_bundle(root / "reduced")
            selected = json.loads(reduced["fault-model.json"])
            self.assertEqual(selected, dict(fault, operation=0))
            self.assertEqual(tool.failure_signature(records), tool.failure_signature(reduced))
            self.assertFalse(tool.replay(root / "reduced", BINARY))

    def test_durable_cut_keeps_committed_content_and_invalid_anchor_refuses(self):
        value = {"version": 1, "volume": fixture()["volume"],
            "operations": [{"op": "create", "label": "f", "parent": "root", "name": "a", "data": "01"}],
            "expected": [{"path": ["a"], "kind": "file", "data": "01"}]}
        records, success = tool.execute(tool.encoded(value), BINARY)
        self.assertTrue(success)
        _, first, last, _, _ = struct.unpack("<IQQQB", records["flight-recorder.bin"][12:41])
        fault = {"version": 1, "kind": "power-cut-v1", "operation": 0, "offset": last - first, "variant": 0}
        records, success = tool.execute(tool.encoded(value), BINARY, fault=fault)
        self.assertTrue(success)
        tool.validate_trace(records)
        for bad in (dict(fault, offset=65536), dict(fault, operation=True), dict(fault, variant=4132)):
            with self.assertRaises(ValueError):
                tool.execute(tool.encoded(value), BINARY, fault=bad)

    def test_checker_findings_are_required_and_preserved_in_the_failure_signature(self):
        records, success = tool.execute(tool.encoded(fixture()), BINARY)
        self.assertTrue(success)
        actual = json.loads(records["actual.json"])
        self.assertEqual(actual["version"], 2)
        self.assertTrue(tool.structural_success(actual))
        self.assertTrue(actual["raw_check"]["clean"])
        self.assertTrue(actual["recovered_check"]["clean"])
        for view in ("raw_check", "recovered_check"):
            damaged = json.loads(records["actual.json"])
            damaged[view]["clean"] = False
            damaged[view]["errors"] = ["reachable block marked free"]
            self.assertFalse(tool.structural_success(damaged))
            edited = dict(records, **{"actual.json": tool.encoded(damaged)})
            signature = json.loads(tool.failure_signature(edited))
            self.assertEqual(signature["kind"], "structure")
            self.assertEqual(signature["findings"][view], ["reachable block marked free"])
        report = actual["raw_check"]
        for field, value in (("schema_version", True), ("clean", 1), ("errors", ["hidden finding"])):
            edited = dict(report, **{field: value})
            with self.assertRaises(ValueError):
                tool.checker_report(tool.encoded(edited).hex())
        without_recovery = dict(actual, recovered_check=None)
        self.assertFalse(tool.structural_success(without_recovery))
        with self.assertRaises(ValueError):
            tool.observation(b"AFSOBS01\nrun ok\nobserve ok\n")

    def test_export_budget_refuses_before_runner(self):
        with self.assertRaisesRegex(ValueError, "per-file budget"):
            tool.execute(tool.encoded(fixture()), Path("/no/such/runner"), file_bytes=4096)
        with self.assertRaisesRegex(ValueError, "aggregate budget"):
            tool.execute(tool.encoded(fixture()), Path("/no/such/runner"), total_bytes=4096)


if __name__ == "__main__":
    unittest.main()
