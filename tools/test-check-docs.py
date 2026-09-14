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


class ProgressTests(unittest.TestCase):
    def run_fixture(self, finite, ongoing="Ongoing", extra=""):
        spec = importlib.util.spec_from_file_location(
            "afsplus_progress", Path(__file__).with_name("progress-markers.py"))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "implementation").mkdir()
            stages = ", ".join(f"Stage {s}" for s in "ABCDEF")
            table = ("| ID | Milestone | Status | Exit criteria | Design | Test plan | Roadmap stages |\n"
                     "|---|---|---|---|---|---|---|\n"
                     f"| M00 | Release | {finite} | gate | design | tests | {stages} |\n"
                     f"| M01 | Review | {ongoing} | ongoing | design | tests | {stages} |\n\n" + extra)
            (root / "implementation/milestones.md").write_text(table)
            for name in ["README.md", "ROADMAP.md", "implementation/README.md",
                         "implementation/implementation-plan.md"]:
                (root / name).write_text("[Stage A](#a) [~~Stage 0~~](#zero)\n")
            before = (root / "README.md").read_text()
            self.assertTrue(module.refresh(root))
            self.assertEqual((root / "README.md").read_text(), before)
            module.refresh(root, write=True)
            self.assertEqual(module.refresh(root), [])
            return (root / "README.md").read_text()

    def test_ongoing_work_does_not_prevent_finite_completion(self):
        self.assertIn("[~~Stage A~~]", self.run_fixture("Complete"))

    def test_unfinished_finite_gate_still_counts(self):
        self.assertIn(r"[\[Stage A\]]", self.run_fixture("Partial"))

    def test_all_ongoing_is_not_completed(self):
        self.assertIn("[Stage A]", self.run_fixture("Ongoing"))

    def test_other_tables_cannot_override_milestone_status(self):
        self.assertIn("[~~Stage A~~]", self.run_fixture(
            "Complete", extra="| M00 | Audit | Not started | notes |\n"))


if __name__ == "__main__":
    unittest.main()
