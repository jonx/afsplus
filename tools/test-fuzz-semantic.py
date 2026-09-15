#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""Independent model examples, deterministic generation and publication controls."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import subprocess
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


class FamilyModelTests(unittest.TestCase):
    def test_window_model_encodes_acknowledged_prefix_commit_and_remount_loss(self):
        model = tool.WindowModel()
        for op in [{"op": "create", "label": "f", "parent": "root", "name": "f", "data": "0102"},
                   {"op": "window_write", "label": "f", "offset": 4, "data": "aa"}, {"op": "window_fsync"},
                   {"op": "window_truncate", "label": "f", "size": 1}, {"op": "remount"}]:
            model.apply(op)
        self.assertEqual(model.committed.expected(), [{"path": ["f"], "kind": "file", "data": "01020000aa"}])
        for op in [{"op": "window_truncate", "label": "f", "size": 1},
                   {"op": "window_write", "label": "f", "offset": 6, "data": "bb"}, {"op": "window_commit"}]:
            model.apply(op)
        self.assertEqual(model.committed.expected()[0]["data"], "010000000000bb")
        model.apply({"op": "window_write", "label": "f", "offset": 0, "data": "cc"})
        for bad in ({"op": "sync"}, {"op": "window_truncate", "label": "f", "size": 7},
                    {"op": "window_write", "label": "f", "offset": 0, "data": ""}):
            with self.assertRaises(ValueError):
                model.apply(bad)
        for _ in range(8):
            model.apply({"op": "window_fsync"})
            model.apply({"op": "window_write", "label": "f", "offset": 1, "data": "dd"})
        with self.assertRaisesRegex(ValueError, "intent log"):
            model.apply({"op": "window_fsync"})

    def test_object_model_links_symlinks_clones_protection_and_moves(self):
        model = tool.ObjectModel()
        for index, op in enumerate([
                {"op": "mkdir", "label": "a", "parent": "root", "name": "a"},
                {"op": "mkdir", "label": "b", "parent": "a", "name": "b"},
                {"op": "create", "label": "f", "parent": "a", "name": "f", "data": "0102"},
                {"op": "link", "label": "l", "source": "f", "parent": "b", "name": "l"},
                {"op": "write", "label": "l", "offset": 3, "data": "ff"},
                {"op": "symlink", "label": "s", "parent": "root", "name": "s", "target": "a/f"},
                {"op": "set_protection", "label": "f", "protection": 9},
                {"op": "clone_file", "label": "c", "source": "l", "parent": "root", "name": "c"},
                {"op": "clone_range", "source": "f", "source_offset": 0, "destination": "c",
                 "destination_offset": 4096, "length": 2},
                {"op": "write", "label": "f", "offset": 0, "data": "ee"},
                {"op": "rename", "label": "b", "parent": "root", "name": "b2"},
                {"op": "unlink", "label": "f"}, {"op": "unlink_symlink", "label": "s"}]):
            model.apply(op, index)
        block = [{"offset": 0, "length": 4096, "unwritten": False}]
        self.assertEqual(model.linked(), [
            {"path": ["a"], "kind": "directory", "links": 1, "protection": 0},
            {"path": ["b2"], "kind": "directory", "links": 1, "protection": 0},
            {"path": ["b2", "l"], "kind": "file", "links": 1, "protection": 9, "alias": 2,
             "policy": False, "data": "ee0200ff", "alloc": block},
            {"path": ["c"], "kind": "file", "links": 1, "protection": 9, "alias": 3, "policy": False,
             "data": "010200ff" + "00" * 4092 + "0102", "alloc": None}])
        self.assertEqual(model.generation, 14)
        model.apply({"op": "mkdir", "label": "inner", "parent": "a", "name": "inner"}, 14)
        for op in ({"op": "rename", "label": "a", "parent": "inner", "name": "loop"},
                   {"op": "rename", "label": "a", "parent": "a", "name": "loop"},
                   {"op": "link", "label": "x", "source": "a", "parent": "root", "name": "x"},
                   {"op": "clone_range", "source": "l", "source_offset": 0, "destination": "l",
                    "destination_offset": 0, "length": 1},
                   {"op": "clone_range", "source": "l", "source_offset": 0, "destination": "c",
                    "destination_offset": 1, "length": 1},
                   {"op": "create", "label": "c", "parent": "root", "name": "again", "data": ""},
                   {"op": "rmdir", "label": "b"}):
            with self.assertRaises(ValueError, msg=op):
                model.apply(op, 20)
        model.apply({"op": "symlink", "label": "t", "parent": "root", "name": "t", "target": "x"}, 21)
        with self.assertRaisesRegex(ValueError, "symlink"):
            model.entries()

    def test_object_model_captures_generation_time_and_single_block_allocation(self):
        model = tool.ObjectModel(single_block=True)
        for index, op in enumerate([
                {"op": "create", "label": "f", "parent": "root", "name": "f", "data": ""},
                {"op": "truncate", "label": "f", "size": 100},
                {"op": "snapshot_create", "label": "s"},
                {"op": "write", "label": "f", "offset": 50, "data": "63"},
                {"op": "snapshot_create", "label": "t"}, {"op": "snapshot_delete", "label": "s"},
                {"op": "mkdir", "label": "d", "parent": "root", "name": "d"},
                {"op": "rename", "label": "f", "parent": "d", "name": "g"}]):
            model.apply(op, index)
        [view] = model.snapshots()
        self.assertEqual((view["id"], view["generation"], view["committed_tx_id"]), (2, 5, 5))
        self.assertEqual(view["root"], {"object_id": 1, "kind": "directory", "size": 0, "allocated": 4096,
            "links": 1, "protection": 0, "created": [0, 0], "modified": [1, 0], "changed": [1, 0],
            "content_generation": 2})
        self.assertEqual(view["entries"], [{"path": ["f"], "data": "00" * 50 + "63" + "00" * 49,
            "allocation": [{"offset": 0, "length": 4096, "unwritten": False}],
            "metadata": {"object_id": 16, "kind": "file", "size": 100, "allocated": 4096, "links": 1,
                         "protection": 0, "created": [1, 0], "modified": [4, 0], "changed": [4, 0],
                         "content_generation": 5}}])
        for op in ({"op": "snapshot_open", "label": "s"}, {"op": "snapshot_close", "label": "t"},
                   {"op": "snapshot_create", "label": "t"}):
            with self.assertRaises(ValueError):
                model.apply(op, 9)
        model.apply({"op": "snapshot_open", "label": "t"}, 9)
        with self.assertRaisesRegex(ValueError, "open"):
            model.apply({"op": "snapshot_delete", "label": "t"}, 10)
        model.apply({"op": "write", "label": "f", "offset": 4096, "data": "01"}, 11)
        with self.assertRaisesRegex(ValueError, "single-block"):
            model.apply({"op": "snapshot_create", "label": "u"}, 12)


class VersionNineModelTests(unittest.TestCase):
    def test_replacement_orphan_lifecycle_and_reserved_directory_totals(self):
        model = tool.ObjectModel(orphan_extents=2)
        for index, op in enumerate([
                {"op": "create", "label": "f", "parent": "root", "name": "f", "data": "0102"},
                {"op": "create", "label": "g", "parent": "root", "name": "g", "data": "03"},
                {"op": "link", "label": "l", "source": "g", "parent": "root", "name": "l"},
                {"op": "rename_replace", "label": "f", "victim": "g", "parent": "root", "name": "g"},
                {"op": "create", "label": "h", "parent": "root", "name": "h", "data": "040506"},
                {"op": "rename_replace_orphan", "label": "h", "victim": "l", "parent": "root", "name": "l"}]):
            model.apply(op, index)
        # The hard-linked victim survived the first replacement; the second
        # replacement moved its final link into the reserved directory.
        self.assertEqual([entry["path"] for entry in model.linked()], [["g"], ["l"]])
        self.assertEqual(model.orphan_state(), {"count": 1, "bytes": 1})
        model.apply({"op": "cleanup_orphan", "label": "l"}, 6)
        self.assertEqual(model.orphan_state(), {"count": 0, "bytes": 0})
        model.apply({"op": "orphan_file", "label": "h"}, 7)
        self.assertEqual(model.orphan_state(), {"count": 1, "bytes": 3})
        for bad in ({"op": "cleanup_orphan", "label": "f"},
                    {"op": "rename_replace", "label": "f", "victim": "f", "parent": "root", "name": "g"},
                    {"op": "orphan_file", "label": "missing"}):
            with self.assertRaises(ValueError, msg=bad):
                model.apply(bad, 8)

    def test_reservation_policy_metadata_and_maintenance_invariance(self):
        model = tool.ObjectModel()
        for index, op in enumerate([
                {"op": "create", "label": "f", "parent": "root", "name": "f", "data": "01"},
                {"op": "preallocate", "label": "f", "offset": 8192, "length": 4096},
                {"op": "set_data_policy", "label": "f", "policy": True},
                {"op": "restore_metadata", "label": "f", "protection": 5, "created": 1,
                 "modified": 2, "changed": 3}]):
            model.apply(op, index)
        [entry] = model.linked()
        self.assertEqual((entry["policy"], entry["protection"], entry["data"]), (True, 5, "01"))
        self.assertEqual(entry["alloc"], [{"offset": 0, "length": 4096, "unwritten": False},
                                          {"offset": 8192, "length": 4096, "unwritten": True}])
        before = model.linked()
        for op in ({"op": "reclaim_step"}, {"op": "snapshot_maintenance_step"}):
            model.apply(op, 4)
        self.assertEqual(model.linked(), before)
        with self.assertRaisesRegex(ValueError, "maintenance"):
            model.apply({"op": "snapshot_create", "label": "s"}, 5)
        # A write consumes the reservation of every block it covers.
        model.apply({"op": "write", "label": "f", "offset": 8192, "data": "02"}, 6)
        self.assertEqual(model.linked()[0]["alloc"],
                         [{"offset": 0, "length": 4096, "unwritten": False},
                          {"offset": 8192, "length": 4096, "unwritten": False}])
        with self.assertRaisesRegex(ValueError, "budget"):
            model.apply({"op": "preallocate_bounded", "label": "f", "offset": 0, "length": 65536,
                         "max_blocks": 2, "max_records": 8}, 7)

    def test_batch_is_atomic_and_a_window_keeps_the_acknowledged_prefix(self):
        model = tool.ObjectModel()
        model.apply({"op": "batch", "items": [
            {"op": "create", "label": "a", "parent": "root", "name": "a", "data": "01"},
            {"op": "create", "label": "b", "parent": "root", "name": "b", "data": "02"}]}, 0)
        model.apply({"op": "batch", "items": [
            {"op": "replace", "label": "a", "victim": "b", "parent": "root", "name": "b"}]}, 1)
        self.assertEqual([entry["path"] for entry in model.linked()], [["b"]])
        model.apply({"op": "window_batch", "items": [
            {"op": "create", "label": "c", "parent": "root", "name": "c", "data": "03"}]}, 2)
        with self.assertRaisesRegex(ValueError, "window is open"):
            model.apply({"op": "sync"}, 3)
        model.apply({"op": "window_fsync"}, 3)
        model.apply({"op": "window_batch", "items": [
            {"op": "create", "label": "d", "parent": "root", "name": "d", "data": "04"}]}, 4)
        model.apply({"op": "remount"}, 5)
        self.assertEqual([entry["path"] for entry in model.linked()], [["b"], ["c"]])
        model.apply({"op": "window_batch", "items": [
            {"op": "rename", "label": "c", "parent": "root", "name": "c2"}]}, 6)
        model.apply({"op": "window_commit"}, 7)
        self.assertEqual([entry["path"] for entry in model.linked()], [["b"], ["c2"]])


class FamilyGenerationTests(unittest.TestCase):
    REQUIRED = {
        "replace": {"create", "mkdir", "write", "link", "sync", "remount", "rename_replace"},
        "orphan": {"create", "mkdir", "write", "sync", "remount", "orphan_file", "cleanup_orphan",
                   "rename_replace_orphan"},
        "space": {"create", "mkdir", "write", "truncate", "sync", "remount", "preallocate",
                  "preallocate_bounded", "set_data_policy", "restore_metadata"},
        "batch": {"create", "mkdir", "sync", "remount", "batch", "window_batch", "window_fsync",
                  "window_commit"},
        "maintenance": {"create", "write", "truncate", "unlink", "sync", "remount", "snapshot_create",
                        "snapshot_open", "snapshot_inspect", "snapshot_close", "snapshot_delete",
                        "reclaim_step", "snapshot_maintenance_step"},
        "captured": {"create", "mkdir", "write", "truncate", "unlink", "sync", "remount", "link", "symlink",
                     "clone_file", "preallocate", "snapshot_create", "snapshot_open", "snapshot_inspect",
                     "snapshot_close", "snapshot_delete"},
        "window": {"create", "mkdir", "sync", "remount", "window_write", "window_truncate", "window_fsync",
                   "window_commit"},
        "snapshot": {"write", "truncate", "rename", "unlink", "rmdir", "remount", "snapshot_create",
                     "snapshot_open", "snapshot_inspect", "snapshot_close", "snapshot_delete"},
        "namespace": {"write", "truncate", "rename", "unlink", "sync", "remount", "link", "symlink",
                      "clone_file", "clone_range", "set_protection", "unlink_symlink"}}

    def test_golden_family_seeds_bounds_and_profile_independence(self):
        golden = {"window": "a75d79b8c1216ca2f3900c43e5ebcf6be523246d23472ed270473a878efafdfd",
                  "snapshot": "77a0304b4bc903a02c6a702aa9d7bbe9e59d9d7d78263efd4b55d95f99a22e1c",
                  "namespace": "0bead7675af4f2be5b29eac8a61bfebbad667f90731dc263d81f14ed16f5f302",
                  "replace": "8345e3ec556f75bdc663b793e123846de37d76f480371508c90d3fd82e41f9a3",
                  "orphan": "8f13dc6544941d1f6bb48f8f851755c1d0a1f63d02fe68ebcafa2532f60fdcea",
                  "space": "6923da614d6266a351adfb45e1aedb9f40ad4499a00524b64a8e75ee7e8ded7b",
                  "batch": "98965b514cee6fe4f219e8e5994ae42e66db5e0689b8feb46d0749c95fd7f9a5",
                  "maintenance": "6908a93d61de329d1f08f1aabfa3f8026de705bd970ebcf3088246417290ebce",
                  "captured": "084969bcfaae7677f2f4b0a081ad9fdc5d75ca2477f0186275587db874ea7940"}
        for family, digest in golden.items():
            value = tool.family_scenario(family, 7, 96, 96, 2)
            self.assertEqual(tool.runner.digest(tool.runner.encoded(value)), digest, family)
        for family, required in self.REQUIRED.items():
            for seed in (0, 1, 7, 42, tool.MASK):
                for steps in (64, 96, 256):
                    operations = tool.generate_family(family, seed, steps)
                    self.assertEqual(len(operations), steps)
                    self.assertLessEqual(required, {op["op"] for op in operations}, (family, seed, steps))
                    for prefix in (steps // 2, steps):
                        cases = [tool.family_scenario(family, seed, steps, prefix, pages) for pages in tool.PROFILES]
                        for case in cases:
                            self.assertEqual(case["version"], tool.FAMILY_VERSIONS[family])
                            self.assertEqual(case["operations"], operations[:prefix] + [{"op": "remount"}])
                            self.assertEqual(tool.expected_state(case), tool.expected_state(cases[0]))

    def test_invalid_family_inputs_refuse(self):
        with self.assertRaises(ValueError):
            tool.generate_family("fault", 1, 64)
        for seed, steps in [(-1, 64), (tool.MASK + 1, 64), (True, 64), (1, 63), (1, 257)]:
            with self.assertRaises(ValueError):
                tool.generate_family("namespace", seed, steps)
        with self.assertRaises(ValueError):
            tool.family_scenario("window", 1, 64, 65, 2)
        with self.assertRaises(ValueError):
            tool.family_scenario("snapshot", 1, 64, 64, 3)

    def test_negative_controls_change_exactly_one_case_of_their_family(self):
        for family, controls in tool.CONTROLS.items():
            original = dict(tool.plan_cases([1], 96, family)[0])
            for control in controls:
                cases, applied = tool.plan_cases([1], 96, family, control)
                self.assertEqual(applied["kind"], control)
                self.assertEqual([name for name, value in cases if value != original[name]], [applied["case"]])

    def test_real_runner_passes_generated_cases_and_fails_each_control(self):
        for family, controls in tool.CONTROLS.items():
            value = tool.family_scenario(family, 42, 64, 64, "unlimited")
            self.assertTrue(tool.runner.execute(tool.runner.encoded(value), BINARY)[1], family)
            for control in controls:
                cases, applied = tool.plan_cases([42], 64, family, control)
                raw = tool.runner.encoded(dict(cases)[applied["case"]])
                self.assertFalse(tool.runner.execute(raw, BINARY)[1], control)


class PublicationTests(unittest.TestCase):
    def records(self):
        # Opaque bundle bodies isolate orchestration controls. Actual filesystem
        # semantics are qualified separately with the real runner below.
        result = {name: b"" for name in tool.runner.bundle.ROLES}
        result["run.json"] = tool.runner.encoded({"source_observed": tool.runner.source_identity(),
            "runner_sha256": tool.runner.executable_digest(BINARY)})
        return result

    def test_family_campaign_binds_family_and_reproduces_only_its_control(self):
        cases, applied = tool.plan_cases([1], 64, "namespace", "protection")
        controlled = tool.runner.encoded(dict(cases)[applied["case"]])
        for failing in (True, False):
            with tempfile.TemporaryDirectory(prefix="afsplus-properties-family-") as temporary:
                output = Path(temporary) / "result"
                execute = lambda raw, binary: (self.records(), not (failing and raw == controlled))
                with patch.object(tool.runner, "execute", side_effect=execute), \
                        contextlib.redirect_stdout(io.StringIO()):
                    self.assertEqual(tool.qualify(output, BINARY, [1], 64, family="namespace",
                                                  control="protection"), not failing)
                recipe = json.loads((output / "recipe.json").read_text())
                self.assertEqual((recipe["version"], recipe["family"], recipe["scenario_version"]),
                                 (2, "namespace", 9))
                self.assertEqual(recipe["negative_control"], applied)
                self.assertEqual(tool.control_reproduced(output), failing)
                tool.runner.bundle.read_bundle(output / applied["case"])
        with tempfile.TemporaryDirectory(prefix="afsplus-properties-family-refusal-") as temporary:
            output = Path(temporary) / "result"
            for family, control in (("window", "link-count"), (None, "window-byte"), ("fault", None)):
                with self.assertRaises(ValueError):
                    tool.qualify(output, BINARY, [1], 64, family=family, control=control)
                self.assertFalse(output.exists())

    def test_fresh_replay_requires_recorded_manifests_and_verdicts(self):
        with tempfile.TemporaryDirectory(prefix="afsplus-properties-replay-") as temporary:
            output = Path(temporary) / "result"
            with patch.object(tool.runner, "execute", return_value=(self.records(), True)), \
                    contextlib.redirect_stdout(io.StringIO()):
                self.assertTrue(tool.qualify(output, BINARY, [1], 64, family="window"))
            calls = []
            def run(command, **kwargs):
                calls.append(command)
                return subprocess.CompletedProcess(command, 0, b"", b"")
            with patch.object(tool.subprocess, "run", side_effect=run), contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(tool.replay_campaign(output, BINARY), 8)
            self.assertTrue(all(command[-2] == "replay" for command in calls))
            with patch.object(tool.subprocess, "run", return_value=subprocess.CompletedProcess([], 2, b"", b"")), \
                    contextlib.redirect_stdout(io.StringIO()), self.assertRaisesRegex(ValueError, "fresh replay"):
                tool.replay_campaign(output, BINARY)
            with patch.object(tool.runner, "executable_digest", return_value="0" * 64), \
                    self.assertRaisesRegex(ValueError, "manifest"):
                tool.replay_campaign(output, BINARY)

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
