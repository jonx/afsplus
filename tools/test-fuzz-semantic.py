#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""Independent model examples, deterministic generation and publication controls."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("properties", Path(__file__).with_name("fuzz-semantic.py"))
tool = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tool)
BINARY = tool.runner.ROOT / "target/debug/afsplus-scenario"


class ModelTests(unittest.TestCase):
    def test_sparse_write_shrink_grow_and_namespace_are_independent_bytes(self):
        model = tool.Model()
        for op in [
            {"op": "mkdir", "label": "d", "parent": "root", "name": "dir"},
            {"op": "create", "label": "f", "parent": "d", "name": "a", "data": "0102"},
            {"op": "write", "label": "f", "offset": 4, "data": "aabb"},
            {"op": "truncate", "label": "f", "size": 5},
            {"op": "truncate", "label": "f", "size": 7},
            {"op": "write", "label": "f", "offset": 30, "data": ""},
            {"op": "rename", "label": "f", "parent": "root", "name": "out"},
            {"op": "rmdir", "label": "d"}, {"op": "remount"}, {"op": "sync"},
        ]: model.apply(op)
        self.assertEqual(model.expected(), [{"path": ["out"], "kind": "file", "data": "01020000aa0000"}])
        model.apply({"op": "unlink", "label": "f"})
        self.assertEqual(model.expected(), [])

    def test_model_rejects_collision_and_nonempty_directory_removal(self):
        model = tool.Model()
        model.apply({"op": "mkdir", "label": "d", "parent": "root", "name": "dir"})
        model.apply({"op": "create", "label": "f", "parent": "d", "name": "a", "data": ""})
        with self.assertRaisesRegex(ValueError, "not empty"):
            model.apply({"op": "rmdir", "label": "d"})
        with self.assertRaisesRegex(ValueError, "collision"):
            model.apply({"op": "create", "label": "g", "parent": "d", "name": "a", "data": ""})
        self.assertEqual(len(model.expected()), 2)

    def test_golden_seed_and_all_families_at_bounds(self):
        value = tool.scenario(7, 96, 96, 2)
        self.assertEqual(tool.runner.digest(tool.runner.encoded(value)),
                         "501438397fd3c77044c94e788ea4cbc7fbfe6cfb50b98433b73a5f249b79bcf2")
        for seed in (0, 1, 7, 42, tool.MASK):
            for steps in (64, 96, 256):
                operations = tool.generate(seed, steps)
                self.assertEqual(len(operations), steps)
                self.assertEqual({op["op"] for op in operations},
                                 {"create", "mkdir", "write", "truncate", "rename", "unlink", "rmdir", "sync", "remount"})
                for prefix in (steps // 2, steps):
                    cases = [tool.scenario(seed, steps, prefix, pages) for pages in tool.PROFILES]
                    for case in cases:
                        self.assertEqual(case["operations"], operations[:prefix] + [{"op": "remount"}])
                        self.assertEqual(case["expected"], cases[0]["expected"])
                        tool.runner.scenario.validate(tool.runner.encoded(case))
                        self.assertLessEqual(len(case["expected"]), 47)
                        self.assertLessEqual(sum(len(e.get("data", "")) // 2 for e in case["expected"]), 40 * 8224)

    def test_invalid_generation_inputs_refuse(self):
        for seed, steps in [(-1, 64), (tool.MASK + 1, 64), (True, 64), (1, 63), (1, 257)]:
            with self.assertRaises(ValueError): tool.generate(seed, steps)
        with self.assertRaises(ValueError): tool.scenario(1, 64, 65, 2)
        with self.assertRaises(ValueError): tool.scenario(1, 64, 64, 3)


class PublicationTests(unittest.TestCase):
    def records(self):
        # Opaque bundle bodies isolate orchestration controls. Actual filesystem
        # semantics are qualified separately with the real runner below.
        result = {name: b"" for name in tool.runner.bundle.ROLES}
        result["run.json"] = tool.runner.encoded({"source_observed": tool.runner.source_identity(),
            "runner_sha256": tool.runner.executable_digest(BINARY)})
        return result

    def test_success_failure_and_overwrite_refusal(self):
        for success in (True, False):
            with tempfile.TemporaryDirectory(prefix="afsplus-properties-") as temporary:
                output = Path(temporary) / "result"
                with patch.object(tool.runner, "execute", return_value=(self.records(), success)), contextlib.redirect_stdout(io.StringIO()):
                    self.assertEqual(tool.qualify(output, BINARY, [1], 64), success)
                value = json.loads((output / "result.json").read_text())
                self.assertEqual(value["outcome"], "pass" if success else "failure")
                self.assertEqual(len(value["cases"]), 8 if success else 1)
                for case in value["cases"]:
                    tool.runner.bundle.read_bundle(output / case["case"])
                before = (output / "result.json").read_bytes()
                with self.assertRaises(FileExistsError): tool.qualify(output, BINARY, [1], 64)
                self.assertEqual((output / "result.json").read_bytes(), before)

    def test_runner_exception_and_payload_exhaustion_retain_recipe(self):
        for budget in (1, 512 * 1024**2):
            with tempfile.TemporaryDirectory(prefix="afsplus-properties-error-") as temporary:
                output = Path(temporary) / "result"
                kwargs = {"return_value": (self.records(), True)} if budget == 1 else {"side_effect": ValueError("runner panic")}
                with patch.object(tool.runner, "execute", **kwargs), self.assertRaises(ValueError):
                    tool.qualify(output, BINARY, [7], 64, budget)
                self.assertTrue((output / "recipe.json").is_file())
                self.assertTrue((output / "seed-7-prefix-32-cache-2.json").is_file())
                self.assertFalse((output / "result.json").exists())
                self.assertEqual(json.loads((output / "error.json").read_text())["outcome"], "incomplete")

    def test_bundle_publication_error_never_claims_success(self):
        with tempfile.TemporaryDirectory(prefix="afsplus-properties-publish-") as temporary:
            output = Path(temporary) / "result"
            with patch.object(tool.runner, "execute", return_value=(self.records(), True)), \
                    patch.object(tool.runner.bundle, "publish", side_effect=OSError("barrier refused")), \
                    self.assertRaises(OSError):
                tool.qualify(output, BINARY, [7], 64)
            self.assertFalse((output / "result.json").exists())
            error = json.loads((output / "error.json").read_text())
            self.assertEqual(error["outcome"], "incomplete")
            self.assertIn("barrier refused", error["error"])

    def test_late_completion_error_remains_incomplete(self):
        real_publish = tool.publish_json
        def fail_after_write(path, value):
            real_publish(path, value)
            if path.name == "result.json":
                raise OSError("late completion barrier")
        with tempfile.TemporaryDirectory(prefix="afsplus-properties-late-") as temporary:
            output = Path(temporary) / "result"
            with patch.object(tool.runner, "execute", return_value=(self.records(), True)), \
                    patch.object(tool, "publish_json", side_effect=fail_after_write), \
                    contextlib.redirect_stdout(io.StringIO()), self.assertRaises(OSError):
                tool.qualify(output, BINARY, [7], 64)
            self.assertTrue((output / "result.json").exists())
            self.assertEqual(json.loads((output / "error.json").read_text())["outcome"], "incomplete")

    def test_nonignored_source_output_is_refused(self):
        with tempfile.TemporaryDirectory(prefix="afsplus-properties-source-") as temporary:
            output = tool.runner.ROOT / Path(temporary).name
            with self.assertRaisesRegex(ValueError, "non-ignored source"):
                tool.qualify(output, BINARY, [1], 64)
            self.assertFalse(output.exists())

    def test_changed_source_or_runner_cannot_complete_a_campaign(self):
        for key in ("source_observed", "runner_sha256"):
            records = self.records()
            identity = json.loads(records["run.json"])
            identity[key] = "changed"
            records["run.json"] = tool.runner.encoded(identity)
            with tempfile.TemporaryDirectory(prefix="afsplus-properties-identity-") as temporary:
                output = Path(temporary) / "result"
                with patch.object(tool.runner, "execute", return_value=(records, True)), \
                        self.assertRaisesRegex(ValueError, "changed"):
                    tool.qualify(output, BINARY, [1], 64)
                self.assertFalse((output / "result.json").exists())
                self.assertEqual(json.loads((output / "error.json").read_text())["outcome"], "incomplete")

    def test_admission_leaves_no_output(self):
        with tempfile.TemporaryDirectory(prefix="afsplus-properties-admission-") as temporary:
            output = Path(temporary) / "result"
            for seeds in ([], [1, 1], [True], [-1], list(range(17))):
                with self.assertRaises(ValueError): tool.qualify(output, BINARY, seeds, 64)
                self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
