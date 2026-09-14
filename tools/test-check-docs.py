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


class ProgressTests(unittest.TestCase):
    def test_decorated_ids_remain_valid_milestone_rows(self):
        for token in ['M01', r'\[M01\]', '~~M01~~']:
            self.assertEqual(checker.MILESTONE_ROW.match(f'| {token} | Reader | Partial |')[1], '01')

    def test_status_transitions_and_idempotent_generation(self):
        spec = importlib.util.spec_from_file_location('progress', Path(__file__).with_name('progress-markers.py'))
        progress = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(progress)
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'implementation').mkdir()
            for name in ['README.md', 'ROADMAP.md', 'implementation/README.md', 'implementation/implementation-plan.md']:
                (root / name).write_text('[M01](milestones.md) [Stage A](ROADMAP.md)\n')
            path = root / 'implementation/milestones.md'
            for status, expected in [('Not started', 'M01'), ('Prototype complete, wire experimental', r'\[M01\]'), ('Complete', '~~M01~~')]:
                path.write_text('| M01 | Reader | ' + status + ' | ' + ', '.join('Stage ' + s for s in 'ABCDEF') + ' |\n')
                progress.refresh(root, True)
                self.assertIn('[' + expected + '](milestones.md)', (root / 'README.md').read_text())
                self.assertEqual(progress.refresh(root), [])


if __name__ == "__main__":
    unittest.main()
