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


class LegacyProgressTests(unittest.TestCase):
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
                path.write_text('| ID | Milestone | Status | Stages |\n| M01 | Reader | ' + status + ' | ' + ', '.join('Stage ' + s for s in 'ABCDEF') + ' |\n')
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


class ItemProgressTests(unittest.TestCase):
    def module(self):
        spec = importlib.util.spec_from_file_location('item_progress', Path(__file__).with_name('progress-markers.py'))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    def test_item_descriptions_follow_task_and_containing_stage(self):
        module = self.module()
        text = ("## \[Stage B\]: architecture\n"
                "### B4. Core structures\n"
                "- ~~[directory tree](crates/tree.rs)~~ <!-- progress: tree -->\n")
        descriptions = module.item_descriptions(text, "ROADMAP.md")
        self.assertEqual(descriptions["tree"], (
            "[directory tree](../crates/tree.rs)",
            "[Stage B / B4. Core structures](../ROADMAP.md#b4-core-structures)"))
        old = "| Item | Status | Evidence |\n|---|---|---|\n| tree | Complete | [proof](proof.md) |\n"
        rendered = module.describe_item_table(old, descriptions)
        self.assertIn("| Task | Stage / phase |", rendered)
        self.assertEqual(module.item_states(rendered), {"tree": 2})
        self.assertEqual(module.describe_item_table(rendered, descriptions), rendered)
        changed = module.item_descriptions(text.replace("directory tree", "bounded directories"), "ROADMAP.md")
        refreshed = module.describe_item_table(rendered, changed)
        self.assertIn("[bounded directories]", refreshed)
        self.assertNotIn("[directory tree]", refreshed)
        phase = module.item_descriptions(
            "## Phase 1: portable reader\n- object read <!-- progress: read -->\n",
            "implementation/implementation-plan.md")
        self.assertEqual(phase["read"][1],
                         "[Phase 1: portable reader](implementation-plan.md#phase-1-portable-reader)")

    def test_item_completion_reopens_and_ongoing_stays_unstruck(self):
        module = self.module()
        text = "- [component](code.rs) <!-- progress: component -->\n"
        complete = module.refresh_items(text, {"component": 2}, set())
        self.assertEqual(complete, "- ~~[component](code.rs)~~ <!-- progress: component -->\n")
        self.assertEqual(module.refresh_items(complete, {"component": 2}, set()), complete)
        for state in (0, 1, None):
            self.assertEqual(module.refresh_items(complete, {"component": state}, set()), text)

    def test_unknown_duplicate_and_missing_evidence_refuse(self):
        module = self.module()
        text = "- component <!-- progress: component -->\n"
        with self.assertRaises(ValueError): module.refresh_items(text, {}, set())
        with self.assertRaises(ValueError): module.refresh_items(text + text, {"component": 2}, set())
        header = "| Item | Status | Evidence |\n|---|---|---|\n"
        for rows in ("| component | Complete | absent |\n",
                     "| component | Invented | [proof](proof.md) |\n",
                     "| component | Complete | [proof](proof.md) |\n" * 2):
            with self.assertRaises(ValueError): module.item_states(header + rows)

    def test_read_only_default_and_orphan_refusal_are_atomic(self):
        module = self.module()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "implementation").mkdir()
            paths = ["README.md", "ROADMAP.md", "implementation/README.md", "implementation/implementation-plan.md"]
            for name in paths: (root / name).write_text("[Stage A](#stage-a)\n")
            roadmap = root / "ROADMAP.md"
            roadmap.write_text("- component <!-- progress: component -->\n")
            owner = root / "implementation/milestones.md"
            table = ("| ID | Milestone | Status | Stages |\n"
                     "| M01 | Component | Complete | Stage A, Stage B, Stage C, Stage D, Stage E, Stage F |\n\n"
                     "| Item | Status | Evidence |\n|---|---|---|\n"
                     "| component | Complete | [proof](proof.md) |\n")
            owner.write_text(table)
            before = roadmap.read_text()
            self.assertIn("ROADMAP.md", module.refresh(root))
            self.assertEqual(roadmap.read_text(), before)
            module.refresh(root, True)
            self.assertEqual(module.refresh(root), [])
            before = {name: (root / name).read_text() for name in paths}
            owner.write_text(table + "| orphan | Complete | [proof](proof.md) |\n")
            with self.assertRaises(ValueError): module.refresh(root, True)
            self.assertEqual({name: (root / name).read_text() for name in paths}, before)


class ScopedStageTests(unittest.TestCase):
    def module(self):
        spec = importlib.util.spec_from_file_location(
            'scoped_progress', Path(__file__).with_name('progress-markers.py'))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    def table(self, rows):
        return self.module().STAGE_GATE_HEADER + '\n|---|---|---|---|---|\n' + rows

    def row(self, key='a-core', status='Complete', stage='Stage A', evidence='[test](test.md)'):
        return f'| {stage} | {key} | Executable core | {status} | {evidence} |\n'

    def test_scoped_completion_does_not_require_shared_milestone_completion(self):
        module = self.module()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'implementation').mkdir()
            for name in ['README.md', 'ROADMAP.md', 'implementation/README.md',
                         'implementation/implementation-plan.md']:
                (root / name).write_text('[Stage A](a) [Stage B](b) [M01](m)\n')
            (root / 'ROADMAP.md').write_text(
                '<!-- stage-gates: Stage A = a-core -->\n[Stage A](a)\n')
            path = root / 'implementation/milestones.md'
            base = ('| ID | Milestone | Status | Stages |\n'
                    '| M01 | Shared reader | Partial | Stage A, Stage B, Stage C, Stage D, Stage E, Stage F |\n\n')
            path.write_text(base + self.table(self.row()))
            before = (root / 'README.md').read_text()
            self.assertTrue(module.refresh(root))
            self.assertEqual((root / 'README.md').read_text(), before)
            module.refresh(root, True)
            text = (root / 'README.md').read_text()
            self.assertIn('[~~Stage A~~]', text)
            self.assertIn(r'[\[Stage B\]]', text)
            self.assertIn(r'[\[M01\]]', text)
            self.assertEqual(module.refresh(root), [])
            path.write_text(base.replace('Partial', 'Complete') + self.table(self.row(status='Partial')))
            module.refresh(root, True)
            text = (root / 'README.md').read_text()
            self.assertIn(r'[\[Stage A\]]', text)
            self.assertIn('[~~M01~~]', text)
            # Invalid ownership must refuse the whole write, retaining navigation.
            path.write_text(base + self.table(self.row(key='missing')))
            with self.assertRaises(ValueError):
                module.refresh(root, True)
            self.assertEqual((root / 'README.md').read_text(), text)

    def test_partial_and_ongoing_gates_have_distinct_completion_semantics(self):
        module = self.module()
        roadmap = '<!-- stage-gates: Stage A = a-core,a-review -->'
        for finite, expected in [('Complete', 2), ('Partial', 1), ('Not started', 0), ('Ongoing', None)]:
            table = self.table(self.row(status=finite) + self.row('a-review', 'Ongoing'))
            self.assertEqual(module.scoped_stage_states(table, roadmap), {'Stage A': expected})

    def test_missing_or_extra_gate_refuses_scope_override(self):
        module = self.module()
        for rows in ['', self.row('other'), self.row() + self.row('extra')]:
            with self.assertRaises(ValueError):
                module.scoped_stage_states(self.table(rows), '<!-- stage-gates: Stage A = a-core -->')

    def test_duplicate_gate_ownership_and_declarations_are_rejected(self):
        module = self.module()
        declaration = '<!-- stage-gates: Stage A = a-core -->'
        for table, roadmap in [
            (self.table(self.row() + self.row()), declaration),
            (self.table(self.row()), declaration + '\n' + declaration),
            (self.table(self.row()), '<!-- stage-gates: Stage A = a-core,a-core -->'),
            (self.table(self.row() + self.row(stage='Stage B')),
             declaration + '\n<!-- stage-gates: Stage B = a-core -->'),
            (self.table(self.row()) + '\n' + self.table(self.row()), declaration),
        ]:
            with self.assertRaises(ValueError):
                module.scoped_stage_states(table, roadmap)

    def test_invalid_status_missing_evidence_and_malformed_scope_fail_closed(self):
        module = self.module()
        declaration = '<!-- stage-gates: Stage A = a-core -->'
        for row in [self.row(status='Done'), self.row(evidence='none'), self.row(stage='Stage Z')]:
            with self.assertRaises(ValueError):
                module.scoped_stage_states(self.table(row), declaration)
        for roadmap in ['', '<!-- stage-gates: Stage A=a-core -->', '<!-- stage-gates: Stage A = a-core, -->']:
            with self.assertRaises(ValueError):
                module.scoped_stage_states(self.table(self.row()), roadmap)


if __name__ == "__main__":
    unittest.main()
