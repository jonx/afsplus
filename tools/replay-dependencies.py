#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""Retain and verify offline Cargo registry dependencies for a bound source tree."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import stat
import subprocess


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


source = module("dependency_source", "replay-source.py")
harness = module("dependency_harness", "afsptest.py")
io = source.io


def inventory(root):
    """Hash regular vendored files without following directory or file links."""
    root = Path(root)
    directory = io._directory(root)
    os.close(directory)
    paths = []
    directories = [root]
    def read_error(error):
        raise error
    for parent, dirs, files in os.walk(root, followlinks=False, onerror=read_error):
        for name in dirs:
            path = Path(parent) / name
            if path.is_symlink():
                raise ValueError("dependency directory symlink")
            directories.append(path)
        paths.extend(Path(parent) / name for name in files)
        if len(paths) > source.MAX_FILES or len(directories) > source.MAX_FILES:
            raise ValueError("dependency path count bound")
    entries = []
    total = 0
    for path in sorted(paths, key=lambda p: os.fsencode(p.relative_to(root))):
        relative = os.fsencode(path.relative_to(root))
        source.path_bytes(relative.hex())
        entry, data = source.read_source(root, relative)
        if entry["kind"] != "file":
            raise ValueError("dependency must be a regular file")
        total += len(data)
        if total > source.TOTAL_BYTES:
            raise ValueError("dependency expanded byte bound")
        entries.append({"path": relative.hex(), "size": len(data), "mode": entry["mode"], "sha256": entry["sha256"]})
    return entries, directories


def lock_digest(root):
    entry, data = source.read_source(root, b"Cargo.lock")
    if entry["kind"] != "file":
        raise ValueError("dependency capture requires a regular Cargo.lock")
    return source.sha(data)


def validate_manifest(manifest):
    fields = {"version", "kind", "source_observed", "lockfile_sha256", "cargo_sha256", "rustc_sha256", "files"}
    if not isinstance(manifest, dict) or set(manifest) != fields or type(manifest["version"]) is not int or manifest["version"] != 1 or manifest["kind"] != "cargo-registry-dependencies-v1":
        raise ValueError("dependency manifest fields or version")
    if len(source.encoded(manifest)) > source.MANIFEST_BYTES:
        raise ValueError("dependency manifest bound")
    def digest(value, lengths=(64,)):
        return isinstance(value, str) and len(value) in lengths and all(c in "0123456789abcdef" for c in value)
    observed = manifest["source_observed"]
    if (not isinstance(observed, dict) or set(observed) != {"revision", "working_tree_sha256"}
            or not digest(observed["revision"], (40, 64)) or not digest(observed["working_tree_sha256"])):
        raise ValueError("dependency source identity")
    for key in ("lockfile_sha256", "cargo_sha256", "rustc_sha256"):
        if not digest(manifest[key]):
            raise ValueError("dependency input digest")
    entries = manifest["files"]
    if not isinstance(entries, list) or not entries or len(entries) > source.MAX_FILES:
        raise ValueError("dependency file count")
    paths = []
    total = 0
    for entry in entries:
        if not isinstance(entry, dict) or set(entry) != {"path", "size", "mode", "sha256"}:
            raise ValueError("dependency file fields")
        paths.append(source.path_bytes(entry["path"]))
        if type(entry["size"]) is not int or not 0 <= entry["size"] <= source.FILE_BYTES:
            raise ValueError("dependency file size")
        if type(entry["mode"]) is not int or not 0 <= entry["mode"] <= 0o777 or not digest(entry["sha256"]):
            raise ValueError("dependency file mode or digest")
        total += entry["size"]
    if total > source.TOTAL_BYTES:
        raise ValueError("dependency expanded byte bound")
    if paths != sorted(set(paths)):
        raise ValueError("dependency paths must be unique and sorted")
    all_paths = set(paths)
    for path in paths:
        parts = path.split(b"/")
        if any(b"/".join(parts[:n]) in all_paths for n in range(1, len(parts))):
            raise ValueError("dependency file ancestor")
    # Every crate directory needs Cargo's registry checksum map and manifest.
    crates = {path.split(b"/")[0] for path in paths}
    if any(crate + b"/.cargo-checksum.json" not in all_paths or crate + b"/Cargo.toml" not in all_paths for crate in crates):
        raise ValueError("dependency crate metadata missing")


def verify(package, root):
    package = Path(package)
    directory = io._directory(package)
    try:
        manifest = json.loads(io._read(directory, "manifest.json", source.MANIFEST_BYTES), object_pairs_hook=io._unique)
    finally:
        os.close(directory)
    validate_manifest(manifest)
    before = harness.source_identity(root)
    if manifest["source_observed"] != before or manifest["lockfile_sha256"] != lock_digest(root):
        raise ValueError("dependency source or lockfile mismatch")
    entries, _ = inventory(package / "vendor")
    if entries != manifest["files"]:
        raise ValueError("dependency inventory integrity mismatch")
    if harness.source_identity(root) != before:
        raise ValueError("dependency source changed during verification")
    return manifest


def cargo_config(package):
    """Fixed source replacement; metadata cannot supply commands or URLs."""
    vendor = str((Path(package) / "vendor").resolve())
    return ["--config", 'source.crates-io.replace-with="afsplus-retained"',
            "--config", "source.afsplus-retained.directory=" + json.dumps(vendor, ensure_ascii=False)]


def capture(root, output, cargo, rustc, cargo_home):
    root = Path(root).resolve()
    output = source.fresh_destination(output, [root, cargo_home]).absolute()
    before = harness.source_identity(root)
    lock = lock_digest(root)
    cargo_hash = harness.executable_digest(cargo)
    rustc_hash = harness.executable_digest(rustc)
    output.mkdir(mode=0o700)
    # Inputs are caller-selected. The command is always offline, locked vendoring.
    env = {key: os.environ[key] for key in ("HOME", "TMPDIR") if key in os.environ}
    env.update(PATH="/usr/bin:/bin:/usr/sbin:/sbin", CARGO_HOME=str(Path(cargo_home).resolve()),
               RUSTC=str(Path(rustc).resolve()), CARGO_NET_OFFLINE="true")
    vendor = output / "vendor"
    config = source.run_command(root, [str(Path(cargo).resolve()), "vendor", "--locked", "--offline", str(vendor)],
                                env=env, label="Cargo vendor", limit=65536)
    # The first profile is the default crates.io registry, with local path
    # dependencies already retained by the source package. Reject other sources.
    expected = ('[source.crates-io]\nreplace-with = "vendored-sources"\n\n'
                '[source.vendored-sources]\ndirectory = ' + json.dumps(str(vendor), ensure_ascii=False))
    if config.decode().strip() != expected:
        raise ValueError("unsupported Cargo source replacement profile")
    entries, directories = inventory(vendor)
    manifest = {"version": 1, "kind": "cargo-registry-dependencies-v1", "source_observed": before,
                "lockfile_sha256": lock, "cargo_sha256": cargo_hash, "rustc_sha256": rustc_hash,
                "files": entries}
    validate_manifest(manifest)
    def check_inputs():
        if (harness.source_identity(root) != before or lock_digest(root) != lock
                or harness.executable_digest(cargo) != cargo_hash or harness.executable_digest(rustc) != rustc_hash):
            raise ValueError("dependency capture inputs changed")
    check_inputs()
    # Cargo owns this fresh output tree. Synchronize all regular files and
    # directories before publishing the manifest; no original source is written.
    for entry in entries:
        path = vendor / os.fsdecode(bytes.fromhex(entry["path"]))
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        try:
            if not stat.S_ISREG(os.fstat(fd).st_mode):
                raise ValueError("dependency file changed kind")
            os.fsync(fd)
        finally:
            os.close(fd)
    for path in reversed(directories):
        directory = io._directory(path)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    if inventory(vendor)[0] != entries:
        raise ValueError("dependency inventory changed during publication")
    check_inputs()
    directory = io._directory(output)
    try:
        os.fsync(directory)
        io._write(directory, "manifest.pending", source.encoded(manifest))
        os.link("manifest.pending", "manifest.json", src_dir_fd=directory,
                dst_dir_fd=directory, follow_symlinks=False)
        os.unlink("manifest.pending", dir_fd=directory)
        os.fsync(directory)
        parent = io._directory(output.parent)
        try:
            os.fsync(parent)
        finally:
            os.close(parent)
    finally:
        os.close(directory)
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    retain = commands.add_parser("capture")
    retain.add_argument("source", type=Path)
    retain.add_argument("output", type=Path)
    retain.add_argument("--cargo", type=Path, required=True)
    retain.add_argument("--rustc", type=Path, required=True)
    retain.add_argument("--cargo-home", type=Path, required=True)
    check = commands.add_parser("verify")
    check.add_argument("package", type=Path)
    check.add_argument("--source-root", type=Path, required=True)
    args = parser.parse_args()
    try:
        if args.command == "capture":
            manifest = capture(args.source, args.output, args.cargo, args.rustc, args.cargo_home)
        else:
            manifest = verify(args.package, args.source_root)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        parser.exit(1, f"dependency package refused: {error}\n")
    print(source.encoded({"source_observed": manifest["source_observed"],
                         "files": len(manifest["files"]), "bytes": sum(entry["size"] for entry in manifest["files"])}).decode().strip())


if __name__ == "__main__":
    main()
