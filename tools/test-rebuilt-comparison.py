#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""Rebuilt-runner comparison, original preservation and publication regressions."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import struct
import sys
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("replay_tests", Path(__file__).with_name("test-afsptest.py"))
fixtures = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixtures)
tool = fixtures.tool
BINARY = fixtures.BINARY


class RebuiltComparisonTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="afsplus-rebuilt-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        # A different executable delegates to the real runner without changing its
        # output. This exercises identity separation, not build attestation.
        self.runner = self.root / "runner"
        self.runner.write_text(f"#!{sys.executable}\nimport os\nos.execv({str(BINARY)!r}, [{str(BINARY)!r}])\n")
        self.runner.chmod(0o700)

    def original(self, value=None, fault=None, name="original"):
        records, success = tool.execute(tool.encoded(value or fixtures.fixture()), BINARY, fault=fault)
        path = self.root / name
        tool.bundle.publish(path, records)
        return path, records, success

    def compare(self, original, output="comparison"):
        return tool.compare_rebuilt(original, self.root / output, self.runner, tool.ROOT)

    def cli(self, original, output):
        return subprocess.run([sys.executable, str(tool.ROOT / "tools/afsptest.py"),
            "--runner", str(self.runner), "compare-rebuilt", str(original), str(output),
            "--source-root", str(tool.ROOT)], capture_output=True, timeout=120)

    def test_all_cache_profiles_preserve_bundles_with_distinct_runner_identity(self):
        for pages in (2, 4, 8, "unlimited"):
            value = fixtures.fixture()
            value["version"] = 2
            value["volume"]["tree_cache_pages"] = pages
            path, records, success = self.original(value, name=f"original-{pages}")
            self.assertTrue(success)
            before = {p.name: p.read_bytes() for p in path.iterdir()}
            output = self.root / f"comparison-{pages}"
            result = self.cli(path, output)
            self.assertEqual(result.returncode, 0, result.stderr.decode())
            report = json.loads((output / "report.json").read_bytes())
            self.assertTrue(report["semantic_artifacts_equal"])
            self.assertFalse(report["runner_identical"])
            self.assertFalse(report["build_provenance_attested"])
            self.assertEqual(report["differing_roles"], [])
            self.assertEqual(tool.bundle.read_bundle(output / "original"), records)
            rebuilt = tool.bundle.read_bundle(output / "rebuilt")
            for entry in report["artifacts"]:
                self.assertEqual(entry["original_sha256"], tool.digest(records[entry["role"]]))
                self.assertEqual(entry["rebuilt_sha256"], tool.digest(rebuilt[entry["role"]]))
            with self.assertRaisesRegex(ValueError, "identity mismatch"):
                tool.replay(path, self.runner)
            self.assertTrue(tool.replay(output / "rebuilt", self.runner))
            self.assertEqual(before, {p.name: p.read_bytes() for p in path.iterdir()})
            self.assertEqual(os.stat(output).st_mode & 0o777, 0o700)
            self.assertEqual(os.stat(output / "report.json").st_mode & 0o777, 0o600)

    def test_internal_flight_is_compared_with_distinct_runner_identity(self):
        for pages in (2, 4, 8, "unlimited"):
            value = fixtures.fixture()
            value.update(version=3, flight_capacity=1)
            value["volume"]["tree_cache_pages"] = pages
            path, records, success = self.original(value, name=f"internal-{pages}")
            self.assertTrue(success)
            report, success = self.compare(path, output=f"paired-{pages}")
            self.assertTrue(success)
            self.assertTrue(report["semantic_artifacts_equal"])
            self.assertFalse(report["runner_identical"])
            # Structurally valid re-sealed diagnostic alteration must still
            # fail exact artifact comparison, despite unchanged recovered files.
            changed = dict(records)
            flight = bytearray(records["flight-recorder.bin"])
            generation_offset = 16 + 29 + 12 + 16
            generation = struct.unpack_from("<Q", flight, generation_offset)[0]
            struct.pack_into("<Q", flight, generation_offset, generation + 1)
            changed["flight-recorder.bin"] = bytes(flight)
            edited = self.root / f"altered-{pages}"
            tool.bundle.publish(edited, changed)
            report, success = self.compare(edited, output=f"different-{pages}")
            self.assertTrue(success)  # Recovered filesystem semantics still pass.
            self.assertFalse(report["semantic_artifacts_equal"])
            self.assertEqual(report["differing_roles"], ["flight-recorder.bin"])
            result = self.cli(edited, self.root / f"different-cli-{pages}")
            self.assertEqual(result.returncode, 2, result.stderr.decode())

    def test_semantic_failures_are_equal_but_never_reported_as_success(self):
        for operation_failure in (False, True):
            value = fixtures.fixture()
            if operation_failure:
                value["operations"].insert(1, {"op": "create", "label": "duplicate", "parent": "root", "name": "café", "data": ""})
            else:
                value["expected"][0]["data"] = "ffff"
            path, _, success = self.original(value, name=f"original-{operation_failure}")
            self.assertFalse(success)
            output = self.root / f"comparison-{operation_failure}"
            result = self.cli(path, output)
            self.assertEqual(result.returncode, 2, result.stderr.decode())
            report = json.loads((output / "report.json").read_bytes())
            self.assertTrue(report["semantic_artifacts_equal"])
            self.assertEqual(report["original_outcome"], "failure")
            self.assertEqual(report["rebuilt_outcome"], "failure")

    def test_selected_crash_preserves_fault_and_result(self):
        value = fixtures.fixture()
        value["expected"] = []
        fault = {"version": 1, "kind": "power-cut-v1", "operation": 0, "offset": 1, "variant": 0}
        path, _, success = self.original(value, fault)
        self.assertTrue(success)
        report, success = self.compare(path)
        self.assertTrue(success)
        self.assertTrue(report["semantic_artifacts_equal"])
        rebuilt = tool.bundle.read_bundle(self.root / "comparison/rebuilt")
        self.assertEqual(json.loads(rebuilt["fault-model.json"]), fault)

    def test_diagnostic_difference_is_retained_and_not_hidden_by_same_outcome(self):
        path, records, _ = self.original()
        rebuilt, success = tool.execute(records["operations.afstrace"], self.runner)
        edited = dict(rebuilt)
        event = bytearray(edited["flight-recorder.bin"])
        event[32] ^= 1  # First event's object identity, not its block range.
        edited["flight-recorder.bin"] = bytes(event)
        tool.validate_trace(edited)
        with patch.object(tool, "execute", return_value=(edited, success)):
            report, passed = self.compare(path)
        self.assertTrue(passed)
        self.assertFalse(report["semantic_artifacts_equal"])
        self.assertEqual(report["differing_roles"], ["flight-recorder.bin"])
        self.assertEqual(tool.bundle.read_bundle(self.root / "comparison/rebuilt"), edited)

    def test_cli_reports_diagnostic_drift_even_when_semantic_verdict_passes(self):
        path, _, _ = self.original()
        self.runner.write_text(f"#!{sys.executable}\n" +
            "import subprocess, sys, struct\n" +
            f"wire = bytearray(subprocess.check_output([{str(BINARY)!r}], input=sys.stdin.buffer.read()))\n" +
            "offset = 8\n" +
            "for _ in range(5):\n" +
            "    length = struct.unpack_from('<H', wire, offset)[0]\n" +
            "    offset += 2\n" +
            "    role = wire[offset:offset + length]\n" +
            "    offset += length\n" +
            "    count = struct.unpack_from('<Q', wire, offset)[0]\n" +
            "    offset += 8\n" +
            "    if role == b'flight-recorder.bin': wire[offset + 32] ^= 1\n" +
            "    offset += count\n" +
            "sys.stdout.buffer.write(wire)\n")
        output = self.root / "drift"
        result = self.cli(path, output)
        self.assertEqual(result.returncode, 2, result.stderr.decode())
        report = json.loads((output / "report.json").read_bytes())
        self.assertEqual(report["rebuilt_outcome"], "pass")
        self.assertFalse(report["semantic_artifacts_equal"])
        self.assertEqual(report["differing_roles"], ["flight-recorder.bin"])

    def test_invalid_metadata_and_source_are_refused_before_execution(self):
        _, records, _ = self.original()
        mutations = [
            lambda m: m.update(outcome="failure"),
            lambda m: m.update(runner_sha256="invalid"),
            lambda m: m.update(source_observed=[]),
            lambda m: m.update(reduction=None),
            lambda m: m["source_observed"].update(revision="0" * 40),
            lambda m: m["source_observed"].update(working_tree_sha256="0" * 64),
        ]
        for index, mutate in enumerate(mutations):
            edited = dict(records)
            meta = json.loads(edited["run.json"])
            mutate(meta)
            edited["run.json"] = tool.encoded(meta)
            path = self.root / f"bad-{index}"
            tool.bundle.publish(path, edited)
            with patch.object(tool, "execute", side_effect=AssertionError("must not execute")):
                with self.assertRaises(ValueError):
                    self.compare(path, f"refused-{index}")
            self.assertFalse((self.root / f"refused-{index}").exists())
        edited = dict(records)
        edited["expected.json"] = b"[]\n"
        with self.assertRaisesRegex(ValueError, "expected state binding"):
            tool.admit_run(edited)

    def test_output_collision_and_overlap_never_modify_originals(self):
        path, _, _ = self.original()
        before = {p.name: p.read_bytes() for p in path.iterdir()}
        link = self.root / "link"
        link.symlink_to(path, target_is_directory=True)
        for output in (path, path / "nested", link, tool.ROOT / "comparison-test-refused"):
            with patch.object(tool, "execute", side_effect=AssertionError("must not execute")):
                with self.assertRaises(ValueError):
                    tool.compare_rebuilt(path, output, self.runner, tool.ROOT)
        self.assertEqual(before, {p.name: p.read_bytes() for p in path.iterdir()})

    def test_source_change_between_admission_and_execution_is_refused(self):
        path, records, _ = self.original()
        rebuilt, success = tool.execute(records["operations.afstrace"], self.runner)
        meta = json.loads(rebuilt["run.json"])
        meta["source_observed"]["working_tree_sha256"] = "0" * 64
        rebuilt["run.json"] = tool.encoded(meta)
        with patch.object(tool, "execute", return_value=(rebuilt, success)):
            with self.assertRaisesRegex(ValueError, "source changed"):
                self.compare(path)
        self.assertFalse((self.root / "comparison").exists())

    def test_publication_failure_has_no_false_success_and_never_overwrites(self):
        path, _, _ = self.original()
        real_write = tool.bundle._write
        def fail_report(directory, name, data):
            if name == "report.pending":
                raise OSError("report write failure")
            return real_write(directory, name, data)
        with patch.object(tool.bundle, "_write", side_effect=fail_report):
            with self.assertRaisesRegex(OSError, "report write failure"):
                self.compare(path)
        output = self.root / "comparison"
        self.assertFalse((output / "report.json").exists())
        tool.bundle.read_bundle(output / "original")
        tool.bundle.read_bundle(output / "rebuilt")
        with self.assertRaisesRegex(ValueError, "already exists"):
            self.compare(path)
        # Fail the final parent barrier: even a readable report is not success.
        real_fsync = tool.os.fsync
        with patch.object(tool.os, "fsync", wraps=real_fsync) as counter:
            self.compare(path, "count-barriers")
            barriers = counter.call_count
        calls = 0
        def fail_last(fd):
            nonlocal calls
            calls += 1
            if calls == barriers:
                raise OSError("final parent barrier")
            return real_fsync(fd)
        with patch.object(tool.os, "fsync", side_effect=fail_last):
            with self.assertRaisesRegex(OSError, "final parent barrier"):
                self.compare(path, "late-failure")
        self.assertTrue((self.root / "late-failure/report.json").is_file())


if __name__ == "__main__":
    unittest.main()
