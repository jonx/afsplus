#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""Dependency retention admission and publication; real builds qualify separately."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("dependency_tool", Path(__file__).with_name("replay-dependencies.py"))
tool = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tool)


class DependencyTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="afsplus-dependency-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.source = self.root / "source"
        self.source.mkdir()
        tool.source.git(self.source, "init", "--quiet", "--template=", isolated=True)
        (self.source / "Cargo.lock").write_text('version = 4\n')
        tool.source.git(self.source, "add", "Cargo.lock")
        tool.source.git(self.source, "-c", "user.name=Dependency Fixture", "-c", "user.email=fixture@example.invalid",
                        "commit", "--quiet", "-m", "Fixture", isolated=True)
        self.cargo_home = self.root / "cache"
        self.cargo_home.mkdir()
        self.cargo = self.root / "cargo"
        self.rustc = self.root / "rustc"
        self.rustc.write_text("fixture compiler identity, not executed\n")
        self.output = self.root / "retained dependencies"
        self.cargo_script()

    def cargo_script(self, extra="", config=True):
        self.cargo.write_text(f"#!{sys.executable}\n" + '''import json, os, pathlib, sys
assert sys.argv[1:4] == ['vendor', '--locked', '--offline']
assert os.environ['CARGO_NET_OFFLINE'] == 'true'
root = pathlib.Path(sys.argv[4])
crate = root / 'fixture'
crate.mkdir(parents=True)
(crate / 'Cargo.toml').write_text('[package]\\nname = "fixture"\\nversion = "1.0.0"\\n')
(crate / 'lib.rs').write_text('pub fn fixture() {}\\n')
(crate / '.cargo-checksum.json').write_text('{"files":{},"package":null}\\n')
''' + extra + "\n" + ('''print('[source.crates-io]\\nreplace-with = "vendored-sources"\\n\\n[source.vendored-sources]\\ndirectory = ' + json.dumps(str(root), ensure_ascii=False))
''' if config else "print('unsupported registry configuration')\n"))
        self.cargo.chmod(0o700)

    def capture(self, output=None):
        return tool.capture(self.source, output or self.output, self.cargo, self.rustc, self.cargo_home)

    def test_fresh_cli_capture_verification_and_relocation(self):
        before = tool.harness.source_identity(self.source)
        result = subprocess.run([sys.executable, str(Path(tool.__file__)), "capture", str(self.source), str(self.output),
            "--cargo", str(self.cargo), "--rustc", str(self.rustc), "--cargo-home", str(self.cargo_home)], capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr.decode())
        manifest = tool.verify(self.output, self.source)
        self.assertEqual(manifest["source_observed"], before)
        self.assertEqual(len(manifest["files"]), 3)
        preserved = (self.output / "manifest.json").read_bytes()
        relocated = self.root / 'relocated "dependencies"'
        self.output.rename(relocated)
        result = subprocess.run([sys.executable, str(Path(tool.__file__)), "verify", str(relocated),
            "--source-root", str(self.source)], capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr.decode())
        self.assertEqual((relocated / "manifest.json").read_bytes(), preserved)
        self.assertEqual(tool.harness.source_identity(self.source), before)
        config = tool.cargo_config(relocated)
        self.assertEqual(config[:2], ["--config", 'source.crates-io.replace-with="afsplus-retained"'])
        self.assertEqual(json.loads(config[-1].split("=", 1)[1]), str((relocated / "vendor").resolve()))
        self.assertEqual(relocated.stat().st_mode & 0o777, 0o700)
        self.assertEqual((relocated / "manifest.json").stat().st_mode & 0o777, 0o600)

    def test_missing_extra_changed_and_mode_changed_files_are_refused(self):
        self.capture()
        file = self.output / "vendor/fixture/lib.rs"
        data = file.read_bytes()
        mode = file.stat().st_mode & 0o777
        for mutation in ("missing", "contents", "mode", "extra"):
            with self.subTest(mutation=mutation):
                extra = file.parent / "extra.rs"
                if mutation == "missing":
                    file.unlink()
                elif mutation == "contents":
                    file.write_bytes(data + b"changed")
                elif mutation == "mode":
                    file.chmod(mode ^ 0o100)
                else:
                    extra.write_bytes(b"extra")
                with self.assertRaisesRegex(ValueError, "integrity mismatch"):
                    tool.verify(self.output, self.source)
                file.write_bytes(data)
                file.chmod(mode)
                if extra.exists():
                    extra.unlink()
        tool.verify(self.output, self.source)

    def test_manifest_path_version_and_size_admission_precedes_inventory(self):
        original = self.capture()
        edits = []
        for raw in (b"../escape", b"/absolute", b".git/config", b"a//b"):
            edited = copy.deepcopy(original)
            edited["files"][0]["path"] = raw.hex()
            edits.append(edited)
        edited = copy.deepcopy(original)
        edited["files"].append(edited["files"][0])
        edits.append(edited)
        for field, value in (("version", True), ("kind", "unknown"), ("source_observed", [])):
            edited = copy.deepcopy(original)
            edited[field] = value
            edits.append(edited)
        edited = copy.deepcopy(original)
        edited["files"][0]["size"] = tool.source.FILE_BYTES + 1
        edits.append(edited)
        for edited in edits:
            (self.output / "manifest.json").write_bytes(tool.source.encoded(edited))
            with patch.object(tool, "inventory", side_effect=AssertionError("must not inventory")):
                with self.assertRaises(ValueError):
                    tool.verify(self.output, self.source)

    def test_source_and_lockfile_binding(self):
        manifest = self.capture()
        (self.source / "Cargo.lock").write_text("changed lockfile\n")
        with self.assertRaisesRegex(ValueError, "source or lockfile mismatch"):
            tool.verify(self.output, self.source)
        manifest["source_observed"] = tool.harness.source_identity(self.source)
        (self.output / "manifest.json").write_bytes(tool.source.encoded(manifest))
        with self.assertRaisesRegex(ValueError, "source or lockfile mismatch"):
            tool.verify(self.output, self.source)

    def test_source_and_compiler_changes_during_capture_are_not_published(self):
        for index, action in enumerate(("pathlib.Path('Cargo.lock').write_text('changed')",
                                        "pathlib.Path(os.environ['RUSTC']).write_text('changed compiler')")):
            self.cargo_script(extra=action)
            output = self.root / f"changed-{index}"
            with self.assertRaisesRegex(ValueError, "inputs changed"):
                self.capture(output)
            self.assertTrue(output.is_dir())
            self.assertFalse((output / "manifest.json").exists())

    def test_source_or_vendor_change_during_synchronization_refuses_completion(self):
        fsync = tool.os.fsync
        for index, target_source in enumerate((True, False)):
            output = self.root / f"synchronization-change-{index}"
            changed = False
            def mutate_after_sync(fd):
                nonlocal changed
                fsync(fd)
                if not changed:
                    target = self.source / "Cargo.lock" if target_source else output / "vendor/fixture/lib.rs"
                    target.write_bytes(b"changed during synchronization")
                    changed = True
            with patch.object(tool.os, "fsync", side_effect=mutate_after_sync):
                with self.assertRaisesRegex(ValueError, "changed"):
                    self.capture(output)
            self.assertTrue(changed)
            self.assertFalse((output / "manifest.json").exists())

    def test_vendor_failure_or_unsupported_replacement_never_publishes(self):
        for index, extra in enumerate(("sys.exit(3)", "")):
            self.cargo_script(extra=extra, config=False)
            output = self.root / f"failed-{index}"
            with self.assertRaises(ValueError):
                self.capture(output)
            self.assertFalse((output / "manifest.json").exists())
            with self.assertRaisesRegex(ValueError, "already exists"):
                self.capture(output)

    def test_symlinks_and_special_files_are_refused(self):
        self.capture()
        file = self.output / "vendor/fixture/lib.rs"
        file.unlink()
        file.symlink_to(self.source / "Cargo.lock")
        with self.assertRaises(ValueError):
            tool.verify(self.output, self.source)
        file.unlink()
        os.mkfifo(file)
        with self.assertRaises(ValueError):
            tool.verify(self.output, self.source)
        file.unlink()
        link = self.output / "vendor/linked"
        link.symlink_to(self.source, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "directory symlink"):
            tool.verify(self.output, self.source)

    def test_byte_and_file_count_limits_refuse_unpublished_capture(self):
        for index, setting in enumerate(("TOTAL_BYTES", "MAX_FILES")):
            output = self.root / f"limited-{index}"
            with patch.object(tool.source, setting, 1):
                with self.assertRaisesRegex(ValueError, "bound"):
                    self.capture(output)
            self.assertFalse((output / "manifest.json").exists())

    def test_overlap_existing_output_and_symlink_collision(self):
        for path in (self.source, self.source / "nested", self.cargo_home / "nested"):
            with self.assertRaises(ValueError):
                self.capture(path)
        self.capture()
        before = (self.output / "manifest.json").read_bytes()
        with self.assertRaisesRegex(ValueError, "already exists"):
            self.capture()
        link = self.root / "link"
        link.symlink_to(self.output, target_is_directory=True)
        with self.assertRaises(ValueError):
            self.capture(link)
        self.assertEqual(before, (self.output / "manifest.json").read_bytes())

    def test_source_change_during_verification_is_refused(self):
        self.capture()
        inventory = tool.inventory
        def changed(root):
            result = inventory(root)
            (self.source / "Cargo.lock").write_text("changed while verifying")
            return result
        with patch.object(tool, "inventory", side_effect=changed):
            with self.assertRaisesRegex(ValueError, "changed during verification"):
                tool.verify(self.output, self.source)

    def test_missing_crate_checksum_metadata_is_not_admitted(self):
        manifest = self.capture()
        manifest["files"] = [entry for entry in manifest["files"] if not bytes.fromhex(entry["path"]).endswith(b".cargo-checksum.json")]
        with self.assertRaisesRegex(ValueError, "crate metadata missing"):
            tool.validate_manifest(manifest)

    def test_publication_failures_preserve_partial_output_and_report_late_error(self):
        write = tool.io._write
        def fail_manifest(directory, name, data):
            if name == "manifest.pending":
                raise OSError("manifest failure")
            return write(directory, name, data)
        with patch.object(tool.io, "_write", side_effect=fail_manifest):
            with self.assertRaisesRegex(OSError, "manifest failure"):
                self.capture()
        self.assertFalse((self.output / "manifest.json").exists())
        fsync = tool.os.fsync
        with patch.object(tool.os, "fsync", wraps=fsync) as counter:
            self.capture(self.root / "count")
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
                self.capture(self.root / "late")
        tool.verify(self.root / "late", self.source)


if __name__ == "__main__":
    unittest.main()
