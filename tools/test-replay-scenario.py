#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
import importlib.util
import json
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("scenario", Path(__file__).with_name("replay-scenario.py"))
scenario = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scenario)


def fixture():
    return {"version": 1, "volume": {"block_size": 4096, "blocks": 256, "region_size": 64, "log_slots": 8},
        "operations": [{"op": "mkdir", "label": "d", "parent": "root", "name": "src"},
            {"op": "create", "label": "f", "parent": "d", "name": "café", "data": "00ff"},
            {"op": "write", "label": "f", "offset": 4, "data": "42"},
            {"op": "truncate", "label": "f", "size": 2},
            {"op": "rename", "label": "f", "parent": "root", "name": "out"},
            {"op": "rmdir", "label": "d"}, {"op": "sync"}, {"op": "remount"}],
        "expected": [{"path": ["out"], "kind": "file", "data": "00ff"}]}


class ScenarioTests(unittest.TestCase):
    def test_v9_linked_commands_labels_and_entries_are_version_bound(self):
        value = fixture()
        value.update(version=9, flight_capacity=32, flight_categories=127, flight_sink=None,
                     snapshot_limits={"max_edit_records": 4096, "max_views": 16, "reclaim_records": 8},
                     expected_snapshots=[], data_policy=True, orphan_extents=2,
                     expected_orphans={"count": 1, "bytes": 3})
        value["volume"]["tree_cache_pages"] = 4
        value["operations"] = [
            {"op": "mkdir", "label": "d", "parent": "root", "name": "dir"},
            {"op": "create", "label": "f", "parent": "d", "name": "f", "data": "0102"},
            {"op": "link", "label": "l", "source": "f", "parent": "root", "name": "alias"},
            {"op": "symlink", "label": "s", "parent": "d", "name": "s", "target": "../ταξί"},
            {"op": "clone_file", "label": "c", "source": "l", "parent": "root", "name": "c"},
            {"op": "clone_range", "source": "f", "source_offset": 1, "destination": "c",
             "destination_offset": 4097, "length": 1},
            {"op": "set_protection", "label": "d", "protection": 4294967295},
            {"op": "rename", "label": "d", "parent": "root", "name": "moved"},
            {"op": "unlink_symlink", "label": "s"}, {"op": "unlink", "label": "f"},
            {"op": "create", "label": "v", "parent": "root", "name": "v", "data": "03"},
            {"op": "rename_replace", "label": "c", "victim": "v", "parent": "root", "name": "v"},
            {"op": "create", "label": "o", "parent": "root", "name": "o", "data": "040506"},
            {"op": "orphan_file", "label": "o"}, {"op": "cleanup_orphan", "label": "o"},
            {"op": "preallocate", "label": "l", "offset": 4096, "length": 8192},
            {"op": "preallocate_bounded", "label": "l", "offset": 0, "length": 1,
             "max_blocks": 2, "max_records": 4},
            {"op": "set_data_policy", "label": "l", "policy": True},
            {"op": "restore_metadata", "label": "l", "protection": 7, "created": 1,
             "modified": 2, "changed": 3},
            {"op": "reclaim_step"}, {"op": "snapshot_maintenance_step"},
            {"op": "batch", "items": [{"op": "create", "label": "b", "parent": "root", "name": "b",
                                       "data": "07"},
                                      {"op": "rename", "label": "l", "parent": "root", "name": "moved2"}]},
            {"op": "window_batch", "items": [{"op": "create", "label": "w", "parent": "root",
                                              "name": "w", "data": "08"}]},
            {"op": "window_fsync"}, {"op": "window_commit"}]
        value["expected"] = [
            {"path": ["alias"], "kind": "file", "links": 1, "protection": 0, "alias": 0,
             "policy": False, "data": "0102", "alloc": None},
            {"path": ["c"], "kind": "file", "links": 1, "protection": 0, "alias": 1, "policy": True,
             "data": "0102", "alloc": [{"offset": 0, "length": 4096, "unwritten": False},
                                       {"offset": 4096, "length": 8192, "unwritten": True}]},
            {"path": ["moved"], "kind": "directory", "links": 1, "protection": 4294967295},
            {"path": ["t"], "kind": "symlink", "links": 1, "protection": 0, "target": "x"}]
        raw = json.dumps(value).encode()
        wire = scenario.compile_commands(raw).decode().splitlines()
        self.assertEqual(wire[:2], ["AFSPSC09", "format 4096 256 64 8 4 32 127 0 none 4096 16 8 1 2"])
        for line in ("link l f root " + "alias".encode().hex(), "symlink s d 73 " + "../ταξί".encode().hex(),
                     "clone_file c l root 63", "clone_range f 1 c 4097 1", "set_protection d 4294967295",
                     "unlink_symlink s", "rename_replace c v root 76", "orphan_file o", "cleanup_orphan o",
                     "preallocate l 4096 8192", "preallocate_bounded l 0 1 2 4", "set_data_policy l 1",
                     "restore_metadata l 7 1 2 3", "reclaim_step", "snapshot_maintenance_step",
                     "batch create:b:root:62:07,rename:l:root:6d6f76656432",
                     "window_batch create:w:root:77:08"):
            self.assertIn(line, wire)
        for version in (7, 8):
            with self.assertRaises(ValueError):
                scenario.validate(json.dumps(dict(value, version=version)).encode())
        for index, changes in ((2, {"source": "d"}), (2, {"source": "missing"}), (3, {"target": ""}),
                               (3, {"target": "a\0b"}), (3, {"target": "x" * 1025}), (4, {"label": "f"}),
                               (5, {"source_offset": True}), (5, {"length": 16 * 1024 * 1024}),
                               (5, {"destination": "s"}), (6, {"protection": 1 << 32}),
                               (6, {"protection": -1}), (8, {"label": "f"}), (9, {"label": "s"})):
            candidate = json.loads(raw)
            candidate["operations"][index].update(changes)
            with self.assertRaises(ValueError, msg=(index, changes)):
                scenario.validate(json.dumps(candidate).encode())
        for index, changes in ((11, {"victim": "c"}), (11, {"victim": "d"}), (14, {"label": "missing"}),
                               (15, {"offset": 16 * 1024 * 1024}), (16, {"max_blocks": 0}),
                               (17, {"policy": 1}), (18, {"created": -1})):
            candidate = json.loads(raw)
            candidate["operations"][index].update(changes)
            with self.assertRaises(ValueError, msg=(index, changes)):
                scenario.validate(json.dumps(candidate).encode())
        for index, items in ((22, [{"op": "delete", "label": "l"}]),
                             (22, [{"op": "replace", "label": "l", "victim": "b", "parent": "root",
                                    "name": "62"}]),
                             (21, []), (21, [{"op": "rename", "label": "d", "parent": "root", "name": "x"}]),
                             (21, [{"op": "create", "label": "z", "parent": "root", "name": "z", "data": ""},
                                   {"op": "delete", "label": "z"}])):
            candidate = json.loads(raw)
            candidate["operations"][index]["items"] = items
            with self.assertRaises(ValueError, msg=(index, items)):
                scenario.validate(json.dumps(candidate).encode())
        for key, bad in (("data_policy", 1), ("orphan_extents", 0), ("orphan_extents", 65),
                         ("expected_orphans", {"count": 1})):
            with self.assertRaises(ValueError, msg=key):
                scenario.validate(json.dumps(dict(value, **{key: bad})).encode())
        for index, changes in ((0, {"links": 0}), (0, {"alias": 4}), (2, {"data": "00"}),
                               (3, {"target": ""}), (3, {"protection": True}), (0, {"policy": 1}),
                               (1, {"alloc": [{"offset": 1, "length": 4096, "unwritten": False}]}),
                               (1, {"alloc": [{"offset": 0, "length": 4096, "unwritten": False},
                                              {"offset": 4096, "length": 4096, "unwritten": False}]})):
            candidate = json.loads(raw)
            candidate["expected"][index].update(changes)
            with self.assertRaises(ValueError, msg=(index, changes)):
                scenario.validate(json.dumps(candidate).encode())
        candidate = json.loads(raw)
        del candidate["expected"][0]["alias"]
        with self.assertRaises(ValueError):
            scenario.validate(json.dumps(candidate).encode())

    def test_v8_lifecycle_commands_and_category_mask_are_version_bound(self):
        value = fixture()
        value.update(version=8, flight_capacity=256, flight_categories=32767, flight_sink=None,
                     snapshot_limits={"max_edit_records": 4096, "max_views": 16,
                                      "reclaim_records": 8},
                     expected_snapshots=[])
        value["volume"]["tree_cache_pages"] = 2
        value["operations"][-1:-1] = [{"op": "verify"}, {"op": "remount_refused"}]
        compiled = scenario.compile_commands(json.dumps(value).encode())
        self.assertTrue(compiled.startswith(
            b"AFSPSC08\nformat 4096 256 64 8 2 256 32767 0 none 4096 16 8\n"))
        self.assertIn(b"\nverify\n", compiled)
        self.assertIn(b"\nremount_refused\n", compiled)
        # Version 8 selects every runtime category bit; version 7 keeps seven.
        for version, mask in ((8, 32768), (8, -1), (8, True), (7, 128), (9, 128)):
            with self.assertRaises(ValueError):
                scenario.validate(json.dumps(dict(value, version=version,
                                                  flight_categories=mask)).encode())
        # The two commands belong to version 8 alone.
        for version in (7, 9):
            broken = dict(value, version=version, flight_categories=127)
            if version == 9:
                broken.update(data_policy=False, orphan_extents=1,
                              expected_orphans={"count": 0, "bytes": 0})
            with self.assertRaises(ValueError):
                scenario.validate(json.dumps(broken).encode())
        for command in ({"op": "verify", "label": "f"}, {"op": "remount_refused", "label": "f"}):
            broken = dict(value, operations=value["operations"] + [command])
            with self.assertRaises(ValueError):
                scenario.validate(json.dumps(broken).encode())

    def test_v7_snapshot_limits_and_labels_are_explicit(self):
        value = fixture()
        value.update(version=7, flight_capacity=256, flight_categories=127, flight_sink=None,
                     snapshot_limits={"max_edit_records": 4096, "max_views": 16, "reclaim_records": 8},
                     expected_snapshots=[])
        value["volume"]["tree_cache_pages"] = 2
        value["operations"] = [{"op": "snapshot_create", "label": "s"},
                               {"op": "snapshot_open", "label": "s"}]
        self.assertTrue(scenario.compile_commands(json.dumps(value).encode()).startswith(
            b"AFSPSC07\nformat 4096 256 64 8 2 256 127 0 none 4096 16 8\n"))
        for key in value["snapshot_limits"]:
            for bad in (0, True, -1, 4097):
                candidate = dict(value, snapshot_limits=dict(value["snapshot_limits"], **{key: bad}))
                with self.assertRaises(ValueError): scenario.validate(json.dumps(candidate).encode())
        for operations in ([{"op": "snapshot_open", "label": "missing"}],
                           [value["operations"][0], value["operations"][0]]):
            with self.assertRaises(ValueError):
                scenario.validate(json.dumps(dict(value, operations=operations)).encode())
        with self.assertRaises(ValueError):
            scenario.validate(json.dumps(dict(value, version=6)).encode())

    def test_v6_object_category_is_version_bound(self):
        value = fixture()
        value.update(version=6, flight_capacity=256, flight_categories=127, flight_sink=None)
        value["volume"]["tree_cache_pages"] = 2
        self.assertTrue(scenario.compile_commands(json.dumps(value).encode()).startswith(b"AFSPSC06\n"))
        for version, mask in ((5, 127), (6, 128), (6, True), (6, -1)):
            with self.assertRaises(ValueError):
                scenario.validate(json.dumps(dict(value, version=version, flight_categories=mask)).encode())

    def test_v5_scope_and_category_bounds_are_versioned(self):
        value = fixture()
        value.update(version=5, flight_capacity=256, flight_categories=63, flight_sink=None)
        value["volume"]["tree_cache_pages"] = 2
        raw = json.dumps(value).encode()
        self.assertTrue(scenario.compile_commands(raw).startswith(
            b"AFSPSC05\nformat 4096 256 64 8 2 256 63 0 none\n"))
        for version, mask in ((4, 16), (4, 63), (5, 64), (5, -1), (5, True)):
            with self.assertRaises(ValueError):
                scenario.validate(json.dumps(dict(value, version=version, flight_categories=mask)).encode())

    def test_deferred_commands_require_v5_and_keep_payload_bounds(self):
        value = fixture()
        value.update(version=5, flight_capacity=256, flight_categories=63, flight_sink=None)
        value["volume"]["tree_cache_pages"] = 2
        value["operations"] = value["operations"][:2] + [
            {"op": "window_write", "label": "f", "offset": 4, "data": "42"},
            {"op": "window_truncate", "label": "f", "size": 2},
            {"op": "window_fsync"}, {"op": "window_commit"}]
        self.assertIn(b"window_write f 4 42\n", scenario.compile_commands(json.dumps(value).encode()))
        with self.assertRaises(ValueError):
            scenario.validate(json.dumps(dict(value, version=4, flight_categories=15)).encode())
        value["operations"][2]["offset"] = scenario.MAX_FILE
        with self.assertRaises(ValueError):
            scenario.validate(json.dumps(value).encode())

    def test_v2_cache_profile_is_explicit_and_compiled_canonically(self):
        for pages in (2, 4, 8, "unlimited"):
            value = fixture()
            value["version"] = 2
            value["volume"]["tree_cache_pages"] = pages
            raw = json.dumps(value).encode()
            self.assertEqual(scenario.validate(raw), value)
            self.assertTrue(scenario.compile_commands(raw).startswith(
                f"AFSPSC02\nformat 4096 256 64 8 {pages}\n".encode()))

    def test_missing_unknown_or_cross_version_cache_profiles_refuse(self):
        for pages in (None, 0, 1, 3, 9, True, 2.0, "2", "02", {}, []):
            value = fixture()
            value["version"] = 2
            if pages is not None: value["volume"]["tree_cache_pages"] = pages
            with self.assertRaises(ValueError): scenario.validate(json.dumps(value).encode())
        value = fixture()
        value["volume"]["tree_cache_pages"] = 2
        with self.assertRaises(ValueError): scenario.validate(json.dumps(value).encode())
    def test_image_operation_ladder_preserves_exact_input(self):
        value = fixture()
        self.assertEqual(scenario.validate(json.dumps(value).encode()), value)

    def test_compiled_protocol_keeps_names_and_binary_data_in_hex_fields(self):
        value = fixture()
        value["operations"][1]["name"] = ".."
        wire = scenario.compile_commands(json.dumps(value).encode())
        self.assertIn(b"create f d 2e2e 00ff\n", wire)
        self.assertTrue(wire.startswith(b"AFSPSC01\nformat 4096 256 64 8\n"))
        self.assertIn(b"write f 4 42\n", wire)
        value["operations"][1]["name"] = "line\n$(ignored)"
        value["operations"][1]["data"] = ""
        wire = scenario.compile_commands(json.dumps(value).encode())
        self.assertEqual(len(wire.splitlines()), len(value["operations"]) + 2)
        self.assertNotIn(b"$(ignored)", wire)
        self.assertIn(b" -\n", wire)

    def test_unknown_commands_fields_and_duplicate_keys_refuse(self):
        for operation in [{"op": "shell", "command": "false"}, {"op": "sync", "command": "false"}]:
            value = fixture(); value["operations"] = [operation]
            with self.assertRaises(ValueError): scenario.validate(json.dumps(value).encode())
        with self.assertRaises(ValueError): scenario.validate(b'{"version":1,"version":1}')

    def test_label_dependencies_and_expected_duplicates_refuse(self):
        for fault in range(4):
            value = fixture()
            if fault == 0: value["operations"][1]["parent"] = "missing"
            elif fault == 1: value["operations"][2]["label"] = "d"
            elif fault == 2: value["operations"].append({"op": "rmdir", "label": "d"})
            else: value["expected"].append(value["expected"][0])
            with self.assertRaises(ValueError): scenario.validate(json.dumps(value).encode())

    def test_geometry_names_payloads_and_ranges_are_bounded(self):
        for fault in range(6):
            value = fixture()
            if fault == 0: value["volume"]["blocks"] = True
            elif fault == 1: value["volume"]["block_size"] = 4097
            elif fault == 2: value["operations"][1]["name"] = "../outside"
            elif fault == 3: value["operations"][1]["data"] = "FF"
            elif fault == 4: value["operations"][2]["offset"] = scenario.MAX_FILE
            else: value["operations"][1]["name"] = "é" * 128
            with self.assertRaises(ValueError): scenario.validate(json.dumps(value).encode())


if __name__ == "__main__": unittest.main()
