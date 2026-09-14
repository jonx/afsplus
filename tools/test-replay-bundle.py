#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""Private fixture tests for replay bundle publication and verification."""
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("bundle", Path(__file__).with_name("replay-bundle.py"))
bundle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bundle)


class BundleTests(unittest.TestCase):
    def artifacts(self):
        return {name: (name + "\0payload").encode() for name in bundle.ROLES}

    def test_deterministic_publication_and_fresh_process_validation(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for name in ["first", "second"]:
                bundle.publish(root / name, self.artifacts())
                self.assertEqual(bundle.read_bundle(root / name), self.artifacts())
            self.assertEqual((root / "first/manifest.json").read_bytes(), (root / "second/manifest.json").read_bytes())
            result = subprocess.run([sys.executable, spec.origin, str(root / "first")], capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn(b"semantic result not evaluated", result.stdout)
            with self.assertRaises(FileExistsError):
                bundle.publish(root / "first", self.artifacts())

    def test_every_artifact_write_failure_withholds_manifest(self):
        for fail_at in range(len(bundle.ROLES) + 1):
            with tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary) / "bundle"
                original = bundle._write
                count = 0
                def fail(directory, name, data):
                    nonlocal count
                    if count == fail_at:
                        raise OSError("injected export failure")
                    count += 1
                    return original(directory, name, data)
                with patch.object(bundle, "_write", side_effect=fail):
                    with self.assertRaises(OSError):
                        bundle.publish(root, self.artifacts())
                self.assertFalse((root / bundle.MANIFEST).exists())
                with self.assertRaises((OSError, ValueError)):
                    bundle.read_bundle(root)

    def test_every_sync_failure_is_reported_with_only_complete_or_absent_manifest(self):
        # Artifact files, artifact directory, pending manifest, publication and parent.
        for fail_at in range(len(bundle.ROLES) + 4):
            with tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary) / "bundle"
                original = bundle.os.fsync
                count = 0
                def fail(fd):
                    nonlocal count
                    current = count
                    count += 1
                    if current == fail_at:
                        raise OSError("injected synchronization failure")
                    return original(fd)
                with patch.object(bundle.os, "fsync", side_effect=fail):
                    with self.assertRaises(OSError):
                        bundle.publish(root, self.artifacts())
                self.assertEqual(count, fail_at + 1)
                if (root / bundle.MANIFEST).exists():
                    self.assertGreaterEqual(fail_at, len(bundle.ROLES) + 2)
                    # Integrity can be complete despite an unacknowledged barrier.
                    self.assertEqual(bundle.read_bundle(root), self.artifacts())
                else:
                    with self.assertRaises((OSError, ValueError)):
                        bundle.read_bundle(root)

    def test_completion_publication_never_replaces_an_existing_name(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "bundle"
            original = bundle.os.link
            def collision(*args, **kwargs):
                (root / bundle.MANIFEST).write_bytes(b"keep existing completion")
                return original(*args, **kwargs)
            with patch.object(bundle.os, "link", side_effect=collision):
                with self.assertRaises(FileExistsError):
                    bundle.publish(root, self.artifacts())
            self.assertEqual((root / bundle.MANIFEST).read_bytes(), b"keep existing completion")

    def test_corrupt_missing_symlink_and_oversized_artifacts_refuse(self):
        for fault in ["corrupt", "missing", "symlink", "oversize"]:
            with tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary) / "bundle"
                bundle.publish(root, self.artifacts())
                target = root / "start.img"
                if fault == "corrupt":
                    data = bytearray(target.read_bytes()); data[0] ^= 1; target.write_bytes(data)
                elif fault == "missing":
                    target.unlink()
                elif fault == "symlink":
                    target.unlink(); target.symlink_to(root / "result.img")
                else:
                    target.write_bytes(target.read_bytes() + b"extra")
                with self.assertRaises((OSError, ValueError)):
                    bundle.read_bundle(root)

    def test_manifest_roles_versions_types_and_limits(self):
        for fault in ["duplicate", "traversal", "version", "size", "missing"]:
            with tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary) / "bundle"
                bundle.publish(root, self.artifacts())
                path = root / bundle.MANIFEST
                manifest = json.loads(path.read_text())
                if fault == "duplicate": manifest["artifacts"][1] = manifest["artifacts"][0]
                elif fault == "traversal": manifest["artifacts"][0]["name"] = "../outside"
                elif fault == "version": manifest["schema_version"] = True
                elif fault == "size": manifest["artifacts"][0]["size"] = -1
                else: manifest["artifacts"].pop()
                path.write_text(json.dumps(manifest))
                with self.assertRaises(ValueError): bundle.read_bundle(root)
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "bundle"
            with self.assertRaises(ValueError): bundle.publish(root, self.artifacts(), total_bytes=1)
            self.assertFalse(root.exists())
            bundle.publish(root, self.artifacts())
            with self.assertRaises(ValueError): bundle.read_bundle(root, total_bytes=1)
            with self.assertRaises(ValueError): bundle.read_bundle(root, file_bytes=1)


if __name__ == "__main__":
    unittest.main()
