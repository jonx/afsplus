#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""Reconstruction protocol regressions; fake builds are distinct from real qualification."""
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import unittest
from unittest.mock import patch


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


tool = module("rebuild_tool", "rebuild-replay.py")
dependency_tests = module("dependency_fixtures", "test-replay-dependencies.py")
replay_tests = module("replay_fixtures", "test-afsptest.py")


class RebuildTests(unittest.TestCase):
    def setUp(self):
        fixture = dependency_tests.DependencyTests()
        fixture.setUp()
        self.addCleanup(fixture.doCleanups)
        self.fixture = fixture
        self.root = fixture.root
        self.source_package = self.root / "source-package"
        fixture.capture()
        tool.source.capture(fixture.source, self.source_package)
        self.tools = self.root / "tools"
        for root in tool.ROOTS:
            (self.tools / root).mkdir(parents=True)
        for role in tool.EXECUTABLES + tool.LIBRARIES:
            path = self.tools / role
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b"fixture identity, not executed\n")
            path.chmod(0o755 if role in tool.EXECUTABLES else 0o644)
        (self.tools / "sdk/System.tbd").write_bytes(b"fixture SDK")
        (self.tools / "sdk/alias").symlink_to("System.tbd")
        (self.tools / "clang-resource/header.h").write_bytes(b"fixture resource")
        self.write_cargo()
        self.host = tool.host_profile() if sys.platform == "darwin" else {
            "uname": ["Darwin", "fixture", "25.6.0", "fixture", "arm64"], "sw_vers": "fixture"}
        self.addCleanup(patch.stopall)
        patch.object(tool, "host_profile", return_value=self.host).start()
        self.seal()
        self.bundles = []
        for pages in (2, 4, 8, "unlimited"):
            value = replay_tests.fixture()
            value["version"] = 2
            value["volume"]["tree_cache_pages"] = pages
            records, passed = tool.harness.execute(tool.source.encoded(value), replay_tests.BINARY,
                                                   source_root=fixture.source)
            self.assertTrue(passed)
            path = self.root / f"bundle-{pages}"
            tool.harness.bundle.publish(path, records)
            self.bundles.append(path)

    def write_cargo(self, altered=""):
        script = f"#!{sys.executable}\n" + '''import os, pathlib, shutil, sys
assert '--frozen' in sys.argv and '--offline' in sys.argv
assert os.environ['CARGO_NET_OFFLINE'] == 'true'
flags = os.environ['CARGO_ENCODED_RUSTFLAGS']
''' + altered + '''
if 'absent-linker' in flags:
    print('clang: error: invalid linker name', file=sys.stderr)
    sys.exit(101)
if not list(pathlib.Path(os.environ['SDKROOT']).iterdir()):
    print("ld: library 'System' not found", file=sys.stderr)
    sys.exit(101)
if any('empty-dependencies' in arg for arg in sys.argv):
    print('error: no matching package named caseless', file=sys.stderr)
    sys.exit(101)
target = pathlib.Path(os.environ['CARGO_TARGET_DIR']) / 'debug'
target.mkdir(parents=True)
''' + f"shutil.copy2({str(replay_tests.BINARY)!r}, target / 'afsplus-scenario')\n" + "print('fixture build, not Rust compilation')\n"
        (self.tools / "rust/bin/cargo").write_text(script)

    def seal(self):
        # Independent small inventory encoder, not the verifier under test.
        entries = []
        for prefix in tool.ROOTS:
            for parent, dirs, files in os.walk(self.tools / prefix, followlinks=False):
                for name in files + [name for name in dirs if (Path(parent) / name).is_symlink()]:
                    path = Path(parent) / name
                    link = path.is_symlink()
                    data = os.fsencode(os.readlink(path)) if link else path.read_bytes()
                    entries.append({"path": os.fsencode(path.relative_to(self.tools)).hex(),
                        "mode": stat.S_IMODE(path.lstat().st_mode), "kind": "link" if link else "file",
                        "size": len(data), "sha256": hashlib.sha256(data).hexdigest()})
        entries.sort(key=lambda entry: bytes.fromhex(entry["path"]))
        manifest = {"version": 1, "kind": "darwin-toolchain-copy-observation-v1", "host_observed": self.host,
                    "scope": "fixture", "regular_bytes": sum(e["size"] for e in entries if e["kind"] == "file"),
                    "entries": entries}
        (self.tools / "copy-manifest.json").write_bytes(tool.source.encoded(manifest))
        return manifest

    def rebuild(self, name="output", bundles=None):
        return tool.rebuild(self.source_package, self.fixture.output, self.tools,
                            bundles or self.bundles, self.root / name)

    def test_seal_and_verify_preserve_payload_and_refuse_replacement(self):
        (self.tools / "copy-manifest.json").unlink()
        before = {str(path): path.read_bytes() for prefix in tool.ROOTS
                  for path in (self.tools / prefix).rglob("*") if path.is_file() and not path.is_symlink()}
        manifest = tool.seal_toolchain(self.tools)
        self.assertEqual(tool.verify_toolchain(self.tools)[0], manifest)
        self.assertEqual(before, {name: Path(name).read_bytes() for name in before})
        with self.assertRaisesRegex(ValueError, "already exists"):
            tool.seal_toolchain(self.tools)

    def test_seal_detects_payload_change_during_barriers(self):
        (self.tools / "copy-manifest.json").unlink()
        fsync = tool.os.fsync
        changed = False
        def mutate(fd):
            nonlocal changed
            fsync(fd)
            if not changed:
                (self.tools / "sdk/System.tbd").write_bytes(b"changed while sealing")
                changed = True
        with patch.object(tool.os, "fsync", side_effect=mutate):
            with self.assertRaisesRegex(ValueError, "changed during sealing"):
                tool.seal_toolchain(self.tools)
        self.assertFalse((self.tools / "copy-manifest.json").exists())

    def test_pipeline_binds_four_profiles_logs_runner_and_unchanged_originals(self):
        before = [{p.name: p.read_bytes() for p in path.iterdir()} for path in self.bundles]
        report = self.rebuild()
        self.assertTrue(report["pass"])
        inputs = json.loads((self.root / "output/build-inputs.json").read_bytes())
        self.assertEqual(set(inputs["environment"]) - {"HOME", "TMPDIR"},
            {"PATH", "CARGO_HOME", "RUSTC", "CARGO_TARGET_DIR", "CARGO_NET_OFFLINE",
             "CARGO_ENCODED_RUSTFLAGS", "SDKROOT", "MACOSX_DEPLOYMENT_TARGET"})
        for key in ("HOME", "TMPDIR"):
            if key in os.environ:
                self.assertEqual(inputs["environment"][key], os.environ[key])
        self.assertEqual(report["controls"], {"build": 0, "linker-negative": 101, "sdk-negative": 101, "dependencies-negative": 101})
        self.assertEqual([item["cache_profile"] for item in report["comparisons"]], [2, 4, 8, "unlimited"])
        for artifact in report["artifacts"]:
            data = (self.root / "output" / artifact["path"]).read_bytes()
            self.assertEqual(len(data), artifact["size"])
            self.assertEqual(hashlib.sha256(data).hexdigest(), artifact["sha256"])
        for index, path in enumerate(self.bundles):
            self.assertEqual(before[index], {p.name: p.read_bytes() for p in path.iterdir()})
        self.assertEqual(json.loads((self.root / "output/qualification.json").read_bytes()), report)
        self.assertEqual(report["qualifier_host"]["python_version"], sys.version)
        for name in tool.DRIVER_FILES:
            self.assertEqual((self.root / "output/driver" / name).read_bytes(),
                             (Path(tool.__file__).parent / name).read_bytes())
        if sys.platform == "darwin":
            verified = subprocess.run([sys.executable, str(self.root / "output/driver/rebuild-replay.py"),
                "verify-toolchain", str(self.tools)], capture_output=True)
            self.assertEqual(verified.returncode, 0, verified.stderr.decode())

    def test_unchanged_failure_is_not_a_successful_qualification(self):
        value = replay_tests.fixture()
        value["expected"][0]["data"] = "ffff"
        records, passed = tool.harness.execute(tool.source.encoded(value), replay_tests.BINARY,
                                               source_root=self.fixture.source)
        self.assertFalse(passed)
        path = self.root / "failure"
        tool.harness.bundle.publish(path, records)
        report = self.rebuild(bundles=[path])
        self.assertFalse(report["pass"])
        self.assertTrue(report["comparisons"][0]["artifacts_equal"])
        self.assertFalse(report["comparisons"][0]["semantic_success"])

    def test_tampered_tools_host_and_inventory_refuse_before_restoration(self):
        original = self.seal()
        for kind in ("contents", "extra", "host"):
            with self.subTest(kind=kind):
                manifest = copy.deepcopy(original)
                target = self.tools / "sdk/System.tbd"
                data = target.read_bytes()
                extra = self.tools / "sdk/extra"
                if kind == "contents":
                    target.write_bytes(b"tampered")
                elif kind == "extra":
                    extra.write_bytes(b"extra")
                else:
                    manifest["host_observed"]["sw_vers"] = "different host"
                    (self.tools / "copy-manifest.json").write_bytes(tool.source.encoded(manifest))
                with patch.object(tool.source, "restore", side_effect=AssertionError("must not restore")):
                    with self.assertRaises(ValueError):
                        self.rebuild(kind)
                self.assertFalse((self.root / kind).exists())
                target.write_bytes(data)
                if extra.exists():
                    extra.unlink()
                (self.tools / "copy-manifest.json").write_bytes(tool.source.encoded(original))

    def test_manifest_paths_duplicates_sizes_and_required_roles(self):
        manifest = self.seal()
        for raw in (b"../escape", b"/absolute", b"sdk/.git/config", b"outside/file"):
            edited = copy.deepcopy(manifest)
            edited["entries"][0]["path"] = raw.hex()
            with self.assertRaises(ValueError):
                tool.admit_tools(edited)
        edited = copy.deepcopy(manifest)
        edited["entries"].append(edited["entries"][0])
        with self.assertRaisesRegex(ValueError, "unique"):
            tool.admit_tools(edited)
        edited = copy.deepcopy(manifest)
        edited["entries"][0]["size"] = tool.FILE_BYTES + 1
        with self.assertRaisesRegex(ValueError, "size"):
            tool.admit_tools(edited)
        edited = copy.deepcopy(manifest)
        removed = next(entry for entry in edited["entries"] if bytes.fromhex(entry["path"]) == b"rust/bin/rustc")
        edited["entries"].remove(removed)
        edited["regular_bytes"] -= removed["size"]
        with self.assertRaisesRegex(ValueError, "required role"):
            tool.admit_tools(edited)

    def test_escaping_symlink_and_special_file_refused_even_with_resealed_path(self):
        alias = self.tools / "sdk/alias"
        alias.unlink()
        alias.symlink_to("../../outside")
        self.seal()
        with self.assertRaises(ValueError):
            tool.verify_toolchain(self.tools)
        alias.unlink()
        alias.symlink_to("System.tbd")
        self.seal()
        target = self.tools / "sdk/System.tbd"
        target.unlink()
        os.mkfifo(target)
        opener = tool.os.open
        def refuse_special_open(path, *args, **kwargs):
            if path == "System.tbd":
                raise AssertionError("special toolchain leaf must be rejected before open")
            return opener(path, *args, **kwargs)
        with patch.object(tool.os, "open", side_effect=refuse_special_open):
            with self.assertRaises(ValueError):
                tool.verify_toolchain(self.tools)

    def test_negative_control_must_fail_for_the_intended_reason(self):
        for number, altered in enumerate((
                "if 'absent-linker' in flags: sys.exit(0)\n",
                "if 'absent-linker' in flags: print('unrelated failure'); sys.exit(2)\n",
                "if 'absent-linker' in flags: print('invalid linker name'); sys.exit(2)\n")):
            self.write_cargo(altered)
            self.seal()
            with self.assertRaisesRegex(ValueError, "negative control"):
                self.rebuild(f"negative-{number}")
            self.assertFalse((self.root / f"negative-{number}/qualification.json").exists())
            self.assertTrue((self.root / f"negative-{number}/linker-negative.log").exists())

    def test_source_binding_refuses_before_output_creation(self):
        manifest, _ = tool.read_json(self.fixture.output, "manifest.json", tool.source.MANIFEST_BYTES)
        manifest["source_observed"]["working_tree_sha256"] = "0" * 64
        (self.fixture.output / "manifest.json").write_bytes(tool.source.encoded(manifest))
        with self.assertRaisesRegex(ValueError, "source/dependency"):
            self.rebuild()
        self.assertFalse((self.root / "output").exists())

    def test_existing_output_and_input_overlap_refused(self):
        for output in (self.source_package, self.fixture.output / "nested", self.tools / "nested", self.bundles[0]):
            with self.assertRaises(ValueError):
                tool.rebuild(self.source_package, self.fixture.output, self.tools, self.bundles, output)
        (self.root / "output").mkdir()
        with self.assertRaisesRegex(ValueError, "already exists"):
            self.rebuild()

    def test_tool_change_after_build_prevents_completion(self):
        original = tool.harness.compare_rebuilt
        def changed(*args, **kwargs):
            result = original(*args, **kwargs)
            (self.tools / "sdk/System.tbd").write_bytes(b"changed after build")
            return result
        with patch.object(tool.harness, "compare_rebuilt", side_effect=changed):
            with self.assertRaisesRegex(ValueError, "integrity mismatch"):
                self.rebuild(bundles=self.bundles[:1])
        self.assertFalse((self.root / "output/qualification.json").exists())

    def test_publication_failure_keeps_evidence_without_claiming_success(self):
        write = tool.io._write
        def fail_completion(directory, name, data):
            if name == "qualification.pending":
                raise OSError("completion failed")
            return write(directory, name, data)
        with patch.object(tool.io, "_write", side_effect=fail_completion):
            with self.assertRaisesRegex(OSError, "completion failed"):
                self.rebuild(bundles=self.bundles[:1])
        self.assertTrue((self.root / "output/comparison-00/report.json").is_file())
        self.assertFalse((self.root / "output/qualification.json").exists())
        with self.assertRaisesRegex(ValueError, "already exists"):
            self.rebuild()

    def test_late_completion_barrier_is_an_error_even_with_readable_report(self):
        fsync = tool.os.fsync
        with patch.object(tool.os, "fsync", wraps=fsync) as counter:
            self.rebuild("count-barriers", bundles=self.bundles[:1])
            barriers = counter.call_count
        calls = 0
        def fail_last(fd):
            nonlocal calls
            calls += 1
            if calls == barriers:
                raise OSError("final parent barrier")
            return fsync(fd)
        with patch.object(tool.os, "fsync", side_effect=fail_last):
            with self.assertRaisesRegex(OSError, "final parent barrier"):
                self.rebuild("late", bundles=self.bundles[:1])
        self.assertTrue((self.root / "late/qualification.json").is_file())

    def test_bounded_command_log_keeps_nonzero_output_and_refuses_overwrite(self):
        log = self.root / "command.log"
        with self.assertRaises(tool.source.CommandFailure) as caught:
            tool.source.run_command(None, [sys.executable, "-c", "import sys; print('out'); print('err', file=sys.stderr); sys.exit(7)"],
                                    merge_stderr=True, log_path=log)
        self.assertEqual(caught.exception.returncode, 7)
        self.assertIn(b"out", log.read_bytes())
        self.assertIn(b"err", log.read_bytes())
        with self.assertRaises(FileExistsError):
            tool.source.run_command(None, [sys.executable, "-c", "print('replacement')"], log_path=log)
        self.assertNotIn(b"replacement", log.read_bytes())


if __name__ == "__main__":
    unittest.main()
