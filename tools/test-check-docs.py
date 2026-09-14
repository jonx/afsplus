#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Temporary-fixture discovery regressions for the documentation checker."""
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("afsplus_check_docs", Path(__file__).with_name("check-docs.py"))
checker = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = checker
spec.loader.exec_module(checker)


class DiscoveryTests(unittest.TestCase):
    def test_excluded_trees_are_never_scanned(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "docs").mkdir()
            (root / "README.md").write_text("# Root\n")
            (root / "docs" / "guide.md").write_text("# Guide\n")
            for parent in [root, root / "docs"]:
                for name in checker.EXCLUDED_TREES:
                    directory = parent / name
                    directory.mkdir()
                    (directory / "excluded.md").write_text("# Excluded\n")
            real_scan = checker.os.scandir

            def guarded_scan(directory):
                self.assertNotIn(Path(directory).name, checker.EXCLUDED_TREES)
                return real_scan(directory)

            with patch.object(checker.os, "scandir", guarded_scan):
                self.assertEqual(checker.markdown_files(root), [root / "README.md", root / "docs" / "guide.md"])

    def test_included_directory_errors_are_reported(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            missing = root / "docs"
            missing.mkdir()
            real_scan = checker.os.scandir

            def disappearing_directory(directory):
                if Path(directory) == missing:
                    raise FileNotFoundError("included directory disappeared")
                return real_scan(directory)

            with patch.object(checker.os, "scandir", disappearing_directory):
                with self.assertRaisesRegex(FileNotFoundError, "included directory"):
                    checker.markdown_files(root)

    def test_discovery_is_sorted_and_does_not_modify_inputs(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for name in ["z.md", "a.md", "ignore.txt"]:
                (root / name).write_text(name)
            self.assertEqual([path.name for path in checker.markdown_files(root)], ["a.md", "z.md"])
            self.assertEqual({path.name: path.read_text() for path in root.iterdir()}, {name: name for name in ["z.md", "a.md", "ignore.txt"]})


if __name__ == "__main__":
    unittest.main()
