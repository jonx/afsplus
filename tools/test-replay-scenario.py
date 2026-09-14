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
