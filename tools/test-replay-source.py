#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""Private source capture/restore with dirty index and hostile-path regressions."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


tool = module("replay_source", "replay-source.py")
harness = module("source_identity_harness", "afsptest.py")


class SourceTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="afsplus-source-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.source = self.root / "source"
        self.source.mkdir()
        tool.git(self.source, "init", "--quiet", "--template=", isolated=True)
        for name, data in {"file": b"committed\n", "deleted": b"gone\n", "stage-delete": b"gone too\n", ".gitignore": b"ignored\n"}.items():
            (self.source / name).write_bytes(data)
        tool.git(self.source, "add", ".")
        tool.git(self.source, "-c", "user.name=Source Fixture", "-c", "user.email=fixture@example.invalid", "commit", "--quiet", "-m", "Fixture", isolated=True)
        self.package = self.root / "package"
        self.restored = self.root / "restored"

    def test_dirty_tree_index_modes_missing_and_untracked_round_trip(self):
        (self.source / "file").write_bytes(b"staged\n")
        tool.git(self.source, "add", "file")
        (self.source / "file").write_bytes(b"working\n")
        (self.source / "file").chmod(0o755)
        (self.source / "new").write_bytes(b"staged new\n")
        tool.git(self.source, "add", "new")
        (self.source / "new").write_bytes(b"unstaged new\n")
        (self.source / "deleted").unlink()
        tool.git(self.source, "rm", "--quiet", "stage-delete")
        (self.source / "untracked").write_bytes(b"untracked data\n")
        (self.source / "ignored").write_bytes(b"must not capture\n")
        (self.source / "link").symlink_to("file")
        odd = "odd\t\ncafé"
        (self.source / odd).write_bytes(b"raw name\n")
        expected = harness.source_identity(self.source)
        original_index = tool.git(self.source, "ls-files", "--stage", "-z")
        status = tool.git(self.source, "status", "--porcelain=v1", "-z")
        self.assertEqual(tool.capture(self.source, self.package), expected)
        before = {p.name: p.read_bytes() for p in self.package.iterdir()}
        self.assertEqual(tool.restore(self.package, self.restored), expected)
        self.assertEqual(harness.source_identity(self.restored), expected)
        self.assertEqual(tool.git(self.restored, "ls-files", "--stage", "-z"), original_index)
        self.assertEqual(tool.git(self.restored, "status", "--porcelain=v1", "-z"), status)
        self.assertEqual((self.restored / "file").read_bytes(), b"working\n")
        self.assertEqual((self.restored / "file").stat().st_mode & 0o777, 0o755)
        self.assertEqual((self.restored / odd).read_bytes(), b"raw name\n")
        self.assertEqual(os.readlink(self.restored / "link"), "file")
        self.assertFalse((self.restored / "deleted").exists())
        self.assertFalse((self.restored / "ignored").exists())
        self.assertEqual(before, {p.name: p.read_bytes() for p in self.package.iterdir()})
        tool.git(self.restored, "fsck", "--full", isolated=True)

    def test_repository_root_with_trailing_newline_is_not_misidentified(self):
        renamed = self.root / "source with newline\n"
        self.source.rename(renamed)
        self.source = renamed
        expected = harness.source_identity(self.source)
        self.assertEqual(tool.capture(self.source, self.package), expected)
        self.assertEqual(tool.restore(self.package, self.restored), expected)

    def test_conflicted_index_preserves_each_stage_and_working_bytes(self):
        objects = [tool.git(self.source, "hash-object", "-w", "--stdin", data=value).strip()
                   for value in (b"base", b"ours", b"theirs")]
        tool.git(self.source, "update-index", "--force-remove", "file")
        index = b"".join(b"100644 " + oid + b" " + str(stage).encode() + b"\tfile\0"
                         for stage, oid in enumerate(objects, 1))
        tool.git(self.source, "update-index", "-z", "--index-info", data=index)
        (self.source / "file").write_bytes(b"unresolved working contents")
        before = tool.git(self.source, "ls-files", "--stage", "-z")
        expected = tool.capture(self.source, self.package)
        self.assertEqual(tool.restore(self.package, self.restored), expected)
        self.assertEqual(tool.git(self.restored, "ls-files", "--stage", "-z"), before)
        self.assertEqual((self.restored / "file").read_bytes(), b"unresolved working contents")

    def test_traversal_reserved_paths_duplicates_and_ancestors_are_rejected(self):
        tool.capture(self.source, self.package)
        manifest, blobs = tool.read_package(self.package)
        for raw in (b"../escape", b"/absolute", b".git/config", b"dir/.GIT/config", b"a//b", b"a/./b", b"a\\b"):
            edited = copy.deepcopy(manifest)
            edited["files"][0]["path"] = raw.hex()
            with self.assertRaises(ValueError):
                tool.admit(edited, blobs)
        edited = copy.deepcopy(manifest)
        edited["files"].append(edited["files"][0])
        with self.assertRaisesRegex(ValueError, "duplicate"):
            tool.admit(edited, blobs)
        edited = copy.deepcopy(manifest)
        child = dict(edited["files"][-1])
        child["path"] = (bytes.fromhex(child["path"]) + b"/child").hex()
        edited["files"].append(child)
        with self.assertRaisesRegex(ValueError, "ancestor"):
            tool.admit(edited, blobs)

    def test_external_symlinks_and_special_files_are_not_captured(self):
        bad = self.source / "bad"
        for target in ("../outside", "/absolute", ".git/config", "a/../../outside"):
            bad.symlink_to(target)
            with self.assertRaises(ValueError):
                tool.capture(self.source, self.package)
            bad.unlink()
            self.assertFalse(self.package.exists())
        bad.write_bytes(b"tracked before becoming special")
        tool.git(self.source, "add", "bad")
        bad.unlink()
        os.mkfifo(bad)
        with self.assertRaisesRegex(ValueError, "file kind"):
            tool.capture(self.source, self.package)
        bad.unlink()
        directory = self.source / "dir"
        directory.mkdir()
        (directory / "file").write_bytes(b"tracked")
        tool.git(self.source, "add", "dir/file")
        (directory / "file").unlink()
        directory.rmdir()
        directory.symlink_to(self.root, target_is_directory=True)
        with self.assertRaises((OSError, ValueError)):
            tool.capture(self.source, self.package)

    def test_blob_corruption_and_symlinked_artifact_fail_before_restore(self):
        tool.capture(self.source, self.package)
        manifest, blobs = tool.read_package(self.package)
        key = manifest["history_sha256"]
        path = self.package / ("blob-" + key)
        data = path.read_bytes()
        path.write_bytes(data[:-1] + bytes([data[-1] ^ 1]))
        with self.assertRaisesRegex(ValueError, "integrity"):
            tool.restore(self.package, self.restored)
        self.assertFalse(self.restored.exists())
        path.unlink()
        external = self.root / "outside"
        external.write_bytes(blobs[key])
        path.symlink_to(external)
        with self.assertRaises(OSError):
            tool.restore(self.package, self.restored)
        self.assertFalse(self.restored.exists())

    def test_bad_index_object_is_refused_before_materialization(self):
        tool.capture(self.source, self.package)
        manifest, blobs = tool.read_package(self.package)
        edited = copy.deepcopy(manifest)
        edited["git_objects"][0]["sha256"] = edited["history_sha256"]
        with self.assertRaisesRegex(ValueError, "index blob identity"):
            tool.admit(edited, blobs)

    def test_file_count_size_and_git_output_bounds(self):
        with patch.object(tool, "MAX_FILES", 1):
            with self.assertRaisesRegex(ValueError, "bound"):
                tool.capture(self.source, self.package)
        with patch.object(tool, "FILE_BYTES", 1):
            with self.assertRaisesRegex(ValueError, "bounded"):
                tool.capture(self.source, self.package)
        with self.assertRaisesRegex(ValueError, "output exceeds"):
            tool.git(self.source, "--version", limit=1)
        self.assertFalse(self.package.exists())

    def test_deduplicated_blobs_cannot_bypass_expanded_worktree_budget(self):
        tool.capture(self.source, self.package)
        manifest, blobs = tool.read_package(self.package)
        sample = next(entry for entry in manifest["files"] if entry["kind"] == "file")
        limit = sum(map(len, blobs.values())) + 100
        for number in range(limit // len(blobs[sample["sha256"]]) + 1):
            entry = dict(sample)
            entry["path"] = f"copy-{number:06d}".encode().hex()
            manifest["files"].append(entry)
        manifest["files"].sort(key=lambda entry: bytes.fromhex(entry["path"]))
        manifest["source_observed"] = tool.identity(manifest["source_observed"]["revision"], manifest["files"])
        with patch.object(tool, "TOTAL_BYTES", limit):
            with self.assertRaisesRegex(ValueError, "expanded source worktree bound"):
                tool.admit(manifest, blobs)

    def test_source_changes_during_capture_are_not_published(self):
        real_scan = tool.scan
        calls = 0
        def changing_scan(root):
            nonlocal calls
            calls += 1
            if calls == 2:
                (root / "file").write_bytes(b"changed during capture")
            return real_scan(root)
        with patch.object(tool, "scan", side_effect=changing_scan):
            with self.assertRaisesRegex(ValueError, "changed during capture"):
                tool.capture(self.source, self.package)
        self.assertFalse(self.package.exists())

    def test_output_collisions_and_overlaps_do_not_overwrite(self):
        tool.capture(self.source, self.package)
        before = {p.name: p.read_bytes() for p in self.package.iterdir()}
        for output in (self.source, self.source / "nested", self.package):
            with self.assertRaises(ValueError):
                tool.capture(self.source, output)
        link = self.root / "link"
        link.symlink_to(self.source, target_is_directory=True)
        for output in (self.package, self.package / "nested", self.source, link):
            with self.assertRaises(ValueError):
                tool.restore(self.package, output)
        self.assertEqual(before, {p.name: p.read_bytes() for p in self.package.iterdir()})

    def test_publication_error_preserves_incomplete_output_and_late_error(self):
        write = tool.io._write
        def fail_manifest(directory, name, data):
            if name == "manifest.pending":
                raise OSError("manifest failure")
            return write(directory, name, data)
        with patch.object(tool.io, "_write", side_effect=fail_manifest):
            with self.assertRaisesRegex(OSError, "manifest failure"):
                tool.capture(self.source, self.package)
        self.assertTrue(self.package.is_dir())
        self.assertFalse((self.package / "manifest.json").exists())
        with self.assertRaisesRegex(ValueError, "already exists"):
            tool.capture(self.source, self.package)
        fsync = tool.os.fsync
        with patch.object(tool.os, "fsync", wraps=fsync) as counter:
            tool.capture(self.source, self.root / "count")
            barriers = counter.call_count
        calls = 0
        def fail_last(fd):
            nonlocal calls
            calls += 1
            if calls == barriers:
                raise OSError("final barrier")
            return fsync(fd)
        with patch.object(tool.os, "fsync", side_effect=fail_last):
            with self.assertRaisesRegex(OSError, "final barrier"):
                tool.capture(self.source, self.root / "late")
        tool.read_package(self.root / "late")


if __name__ == "__main__":
    unittest.main()
