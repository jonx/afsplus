#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""Verify retained Darwin tools and reconstruct private replay from preserved inputs."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import stat
import subprocess
import sys


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


deps = module("rebuild_dependencies", "replay-dependencies.py")
source, harness, io = deps.source, deps.harness, deps.io
ROOTS = ("rust", "apple", "sdk", "clang-resource")
EXECUTABLES = ("rust/bin/cargo", "rust/bin/rustc", "apple/bin/clang", "apple/bin/ld")
LIBRARIES = tuple("apple/lib/" + name for name in
                  ("libtapi.dylib", "libcodedirectory.dylib", "libLTO.dylib", "libswiftDemangle.dylib"))
MANIFEST_BYTES = 32 * 1024 * 1024
FILE_BYTES = 1024 * 1024 * 1024
TOTAL_BYTES = 4 * 1024 * 1024 * 1024
MAX_ENTRIES = 100000
DRIVER_FILES = ("rebuild-replay.py", "replay-source.py", "replay-dependencies.py",
                "afsptest.py", "replay-bundle.py", "replay-scenario.py")


def read_json(root, name, limit):
    directory = io._directory(root)
    try:
        raw = io._read(directory, name, limit)
    finally:
        os.close(directory)
    return json.loads(raw, object_pairs_hook=io._unique), source.sha(raw)


def host_profile():
    return {"uname": list(os.uname()), "sw_vers": source.run_command(None,
        ["/usr/bin/sw_vers"], limit=4096, label="Host observation").decode()}


def admit_tools(manifest):
    fields = {"version", "kind", "host_observed", "scope", "regular_bytes", "entries"}
    if (not isinstance(manifest, dict) or set(manifest) != fields or type(manifest["version"]) is not int
            or manifest["version"] != 1 or manifest["kind"] != "darwin-toolchain-copy-observation-v1"):
        raise ValueError("toolchain manifest fields or version")
    if not isinstance(manifest["scope"], str) or len(manifest["scope"]) > 1024:
        raise ValueError("toolchain scope field")
    host = manifest["host_observed"]
    if (not isinstance(host, dict) or set(host) != {"uname", "sw_vers"}
            or not isinstance(host["uname"], list) or len(host["uname"]) != 5
            or any(not isinstance(value, str) or len(value) > 4096 for value in host["uname"])
            or host["uname"][0] != "Darwin" or host["uname"][4] != "arm64"
            or not isinstance(host["sw_vers"], str) or len(host["sw_vers"]) > 4096):
        raise ValueError("unsupported toolchain host profile")
    entries = manifest["entries"]
    if not isinstance(entries, list) or not entries or len(entries) > MAX_ENTRIES:
        raise ValueError("toolchain entry count")
    paths = []
    total = 0
    for entry in entries:
        if not isinstance(entry, dict) or set(entry) != {"path", "mode", "kind", "size", "sha256"}:
            raise ValueError("toolchain entry fields")
        path = source.path_bytes(entry["path"])
        if path.split(b"/")[0] not in tuple(name.encode() for name in ROOTS) or b"/" not in path:
            raise ValueError("toolchain path outside retained roots")
        paths.append(path)
        if entry["kind"] not in ("file", "link") or type(entry["mode"]) is not int or not 0 <= entry["mode"] <= 0o777:
            raise ValueError("toolchain file kind or mode")
        if type(entry["size"]) is not int or not 0 <= entry["size"] <= (4096 if entry["kind"] == "link" else FILE_BYTES):
            raise ValueError("toolchain entry size")
        digest = entry["sha256"]
        if not isinstance(digest, str) or len(digest) != 64 or any(c not in "0123456789abcdef" for c in digest):
            raise ValueError("toolchain entry digest")
        if entry["kind"] == "file":
            total += entry["size"]
    if paths != sorted(set(paths)):
        raise ValueError("toolchain paths must be unique and sorted")
    if type(manifest["regular_bytes"]) is not int or total != manifest["regular_bytes"] or total > TOTAL_BYTES:
        raise ValueError("toolchain aggregate byte bound")
    all_paths = set(paths)
    for path in paths:
        parts = path.split(b"/")
        if any(b"/".join(parts[:n]) in all_paths for n in range(1, len(parts))):
            raise ValueError("toolchain file ancestor")
    by_path = {os.fsdecode(path): entry for path, entry in zip(paths, entries)}
    for path in EXECUTABLES + LIBRARIES:
        if path not in by_path or by_path[path]["kind"] != "file":
            raise ValueError("toolchain required role missing")
        if path in EXECUTABLES and not by_path[path]["mode"] & 0o111:
            raise ValueError("toolchain executable role lacks execute permission")
    return entries


def tool_entry(root, relative):
    raw = source.path_bytes(os.fsencode(relative).hex())
    directory = io._directory(root)
    try:
        parts = os.fsdecode(raw).split("/")
        for part in parts[:-1]:
            child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=directory)
            os.close(directory)
            directory = child
        info = os.stat(parts[-1], dir_fd=directory, follow_symlinks=False)
        mode = stat.S_IMODE(info.st_mode)
        if mode & ~0o777:
            raise ValueError("unsupported toolchain special mode")
        if stat.S_ISLNK(info.st_mode):
            data = os.fsencode(os.readlink(parts[-1], dir_fd=directory))
            # Each retained component is self-contained; never follow SDK aliases
            # into another component or installed host files.
            source.safe_link(b"/".join(raw.split(b"/")[1:]), data)
            component = (Path(root) / parts[0]).resolve()
            try:
                resolved = (Path(root) / relative).resolve()
            except RuntimeError as error:
                raise ValueError("toolchain symlink cycle") from error
            if resolved != component and component not in resolved.parents:
                raise ValueError("toolchain symlink escapes component")
            return {"path": raw.hex(), "mode": mode, "kind": "link", "size": len(data), "sha256": source.sha(data)}
        if not stat.S_ISREG(info.st_mode):
            raise ValueError("toolchain must be a bounded regular file")
        fd = os.open(parts[-1], os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=directory)
        with os.fdopen(fd, "rb") as stream:
            observed = os.fstat(stream.fileno())
            if not stat.S_ISREG(observed.st_mode) or observed.st_size > FILE_BYTES:
                raise ValueError("toolchain must be a bounded regular file")
            digest = hashlib.sha256()
            size = 0
            while chunk := stream.read(1024 * 1024):
                size += len(chunk)
                if size > FILE_BYTES:
                    raise ValueError("toolchain file grew past bound")
                digest.update(chunk)
        return {"path": raw.hex(), "mode": mode, "kind": "file", "size": size, "sha256": digest.hexdigest()}
    finally:
        os.close(directory)


def tool_paths(root):
    paths = []
    count_dirs = 0
    def read_error(error):
        raise error
    for name in ROOTS:
        component = Path(root) / name
        directory = io._directory(component)
        os.close(directory)
        for parent, dirs, files in os.walk(component, followlinks=False, onerror=read_error):
            count_dirs += 1
            paths.extend((Path(parent) / item).relative_to(root) for item in files)
            paths.extend((Path(parent) / item).relative_to(root) for item in dirs if (Path(parent) / item).is_symlink())
            if len(paths) > MAX_ENTRIES or count_dirs > MAX_ENTRIES:
                raise ValueError("toolchain tree count bound")
    return sorted(paths, key=os.fsencode)


def verify_toolchain(root):
    root = Path(root).resolve()
    manifest, digest = read_json(root, "copy-manifest.json", MANIFEST_BYTES)
    entries = admit_tools(manifest)
    observed = host_profile()
    expected = manifest["host_observed"]
    if (any(observed["uname"][index] != expected["uname"][index] for index in (0, 2, 4))
            or observed["sw_vers"] != expected["sw_vers"]):
        raise ValueError("toolchain host profile mismatch")
    paths = tool_paths(root)
    if [os.fsencode(path).hex() for path in paths] != [entry["path"] for entry in entries]:
        raise ValueError("toolchain path inventory mismatch")
    for path, expected_entry in zip(paths, entries):
        if tool_entry(root, path) != expected_entry:
            raise ValueError("toolchain file integrity mismatch: " + os.fsdecode(path))
    return manifest, digest


def seal_toolchain(root):
    """Publish an inventory of caller-prepared copies; do not copy or execute them."""
    root = Path(root).resolve()
    for name in ("copy-manifest.json", "copy-manifest.pending"):
        if os.path.lexists(root / name):
            raise ValueError("toolchain manifest already exists")
    host = host_profile()
    entries = []
    total = 0
    for path in tool_paths(root):
        entry = tool_entry(root, path)
        entries.append(entry)
        if entry["kind"] == "file":
            total += entry["size"]
        if total > TOTAL_BYTES:
            raise ValueError("toolchain aggregate byte bound")
    manifest = {"version": 1, "kind": "darwin-toolchain-copy-observation-v1", "host_observed": host,
        "scope": "retained Rust, selected Apple linker closure, clang resources and SDK; named macOS runtime prerequisite",
        "regular_bytes": total, "entries": entries}
    admit_tools(manifest)
    if len(source.encoded(manifest)) > MANIFEST_BYTES:
        raise ValueError("toolchain manifest byte bound")
    directories = {root, *(root / name for name in ROOTS)}
    for entry in entries:
        path = root / os.fsdecode(bytes.fromhex(entry["path"]))
        if entry["kind"] == "file":
            fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
            try:
                if not stat.S_ISREG(os.fstat(fd).st_mode):
                    raise ValueError("toolchain file changed kind")
                os.fsync(fd)
            finally:
                os.close(fd)
        parent = path.parent
        while parent != root:
            directories.add(parent)
            parent = parent.parent
    for path in sorted(directories, key=lambda value: len(value.parts), reverse=True):
        directory = io._directory(path)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    if ([tool_entry(root, path) for path in tool_paths(root)] != entries or host_profile() != host):
        raise ValueError("toolchain changed during sealing")
    save(root, "copy-manifest.pending", manifest)
    directory = io._directory(root)
    try:
        os.link("copy-manifest.pending", "copy-manifest.json", src_dir_fd=directory,
                dst_dir_fd=directory, follow_symlinks=False)
        os.unlink("copy-manifest.pending", dir_fd=directory)
        os.fsync(directory)
        parent = io._directory(root.parent)
        try:
            os.fsync(parent)
        finally:
            os.close(parent)
    finally:
        os.close(directory)
    return manifest


def build_inputs(root, toolchain, dependencies, case="build"):
    root, toolchain = Path(root).resolve(), Path(toolchain).resolve()
    home = root / (case + "-cargo-home")
    home.mkdir(mode=0o700)
    flags = ["--sysroot", str(toolchain / "rust"), "-C", "linker=" + str(toolchain / "apple/bin/clang"),
        "-C", "link-arg=--ld-path=" + str(toolchain / "apple/bin/ld"), "-C", "link-arg=-isysroot",
        "-C", "link-arg=" + str(toolchain / "sdk"), "-C", "link-arg=-resource-dir",
        "-C", "link-arg=" + str(toolchain / "clang-resource")]
    if case == "linker-negative":
        flags[5] = "link-arg=--ld-path=" + str(root / "absent-linker")
    sdk = toolchain / "sdk"
    if case == "sdk-negative":
        sdk = root / "empty-sdk"
        sdk.mkdir()
        flags[9] = "link-arg=" + str(sdk)
    if case == "dependencies-negative":
        dependencies = root / "empty-dependencies"
        (dependencies / "vendor").mkdir(parents=True)
    env = {key: os.environ[key] for key in ("HOME", "TMPDIR") if key in os.environ}
    env.update(PATH="/usr/bin:/bin:/usr/sbin:/sbin", CARGO_HOME=str(home),
        RUSTC=str(toolchain / "rust/bin/rustc"), CARGO_TARGET_DIR=str(root / (case + "-target")),
        CARGO_NET_OFFLINE="true", CARGO_ENCODED_RUSTFLAGS="\x1f".join(flags),
        SDKROOT=str(sdk), MACOSX_DEPLOYMENT_TARGET="11.0")
    command = [str(toolchain / "rust/bin/cargo"), *deps.cargo_config(dependencies),
               "build", "--frozen", "--offline", "-p", "afsplus-check", "--bin", "afsplus-scenario", "-vv"]
    return command, env


def save(root, name, value):
    directory = io._directory(root)
    try:
        io._write(directory, name, source.encoded(value))
    finally:
        os.close(directory)


def driver_sources():
    directory = io._directory(Path(__file__).resolve().parent)
    try:
        return {name: io._read(directory, name, 1024 * 1024) for name in DRIVER_FILES}
    finally:
        os.close(directory)


def rebuild(source_package, dependency_package, toolchain, bundles, output):
    if not 1 <= len(bundles) <= 16:
        raise ValueError("reconstruction bundle count bound")
    output = source.fresh_destination(output, [source_package, dependency_package, toolchain, *bundles]).resolve()
    driver = driver_sources()
    tools_manifest, tools_digest = verify_toolchain(toolchain)
    source_manifest, _ = source.read_package(source_package)
    source_digest = read_json(source_package, "manifest.json", source.MANIFEST_BYTES)[1]
    dependency_manifest, dependency_digest = read_json(dependency_package, "manifest.json", source.MANIFEST_BYTES)
    deps.validate_manifest(dependency_manifest)
    identity = source_manifest["source_observed"]
    if dependency_manifest["source_observed"] != identity:
        raise ValueError("reconstruction source/dependency identity mismatch")
    original_digests = []
    original_manifest_digests = []
    profiles = []
    for path in bundles:
        records = harness.bundle.read_bundle(path)
        meta, _ = harness.admit_run(records)
        if meta["source_observed"] != identity:
            raise ValueError("reconstruction bundle source mismatch")
        original_digests.append({role: source.sha(data) for role, data in records.items()})
        original_manifest_digests.append(read_json(path, "manifest.json", 65536)[1])
        value = harness.scenario.validate(records["operations.afstrace"])
        profiles.append(value["volume"].get("tree_cache_pages", "version-1-unlimited"))
    output.mkdir(mode=0o700)
    (output / "driver").mkdir(mode=0o700)
    directory = io._directory(output / "driver")
    try:
        for name, data in driver.items():
            io._write(directory, name, data)
        os.fsync(directory)
    finally:
        os.close(directory)
    qualifier_host = {"python_version": sys.version,
                      "python_executable_sha256": harness.executable_digest(sys.executable),
                      "git_version": source.git(None, "--version").decode().strip()}
    restored = output / "source"
    if source.restore(source_package, restored) != identity:
        raise ValueError("reconstruction restored source mismatch")
    if deps.verify(dependency_package, restored) != dependency_manifest:
        raise ValueError("reconstruction dependencies changed")
    controls = {}
    for case, expected in (("build", None), ("linker-negative", b"invalid linker name"),
                           ("sdk-negative", b"library 'System' not found"),
                           ("dependencies-negative", b"no matching package named")):
        command, env = build_inputs(output, toolchain, dependency_package, case)
        save(output, case + "-inputs.json", {"command": command,
            "environment": env,
            "cargo_home_initially_empty": True})
        try:
            source.run_command(restored, command, env=env, label="Cargo build", timeout=300,
                merge_stderr=True, log_path=output / (case + ".log"))
            if expected is not None:
                raise ValueError("reconstruction negative control unexpectedly passed: " + case)
            controls[case] = 0
        except source.CommandFailure as error:
            if expected is None:
                raise
            directory = io._directory(output)
            try:
                log = io._read(directory, case + ".log", source.MANIFEST_BYTES)
            finally:
                os.close(directory)
            if error.returncode != 101 or expected not in log:
                raise ValueError("reconstruction negative control failed for another reason: " + case) from error
            controls[case] = error.returncode
    reports = []
    for index, path in enumerate(bundles):
        report, success = harness.compare_rebuilt(path, output / f"comparison-{index:02d}",
            output / "build-target/debug/afsplus-scenario", restored)
        captured = harness.bundle.read_bundle(path)
        if ({role: source.sha(data) for role, data in captured.items()} != original_digests[index]
                or read_json(path, "manifest.json", 65536)[1] != original_manifest_digests[index]):
            raise ValueError("reconstruction original bundle changed")
        reports.append({"directory": f"comparison-{index:02d}", "semantic_success": success,
                        "artifacts_equal": report["semantic_artifacts_equal"], "cache_profile": profiles[index]})
    if (driver_sources() != driver or verify_toolchain(toolchain)[1] != tools_digest
            or deps.verify(dependency_package, restored) != dependency_manifest
            or source.read_package(source_package)[0] != source_manifest
            or read_json(source_package, "manifest.json", source.MANIFEST_BYTES)[1] != source_digest
            or read_json(dependency_package, "manifest.json", source.MANIFEST_BYTES)[1] != dependency_digest):
        raise ValueError("reconstruction retained inputs changed")
    artifact_names = [case + suffix for case in controls for suffix in ("-inputs.json", ".log")]
    artifact_names += [item["directory"] + "/report.json" for item in reports]
    artifact_names.append("build-target/debug/afsplus-scenario")
    artifact_names += ["driver/" + name for name in DRIVER_FILES]
    artifacts = [{"path": name, "sha256": harness.executable_digest(output / name),
                  "size": (output / name).stat().st_size} for name in sorted(artifact_names)]
    result = {"version": 1, "kind": "darwin-replay-reconstruction-v1", "source_observed": identity,
        "source_manifest_sha256": source_digest, "dependency_manifest_sha256": dependency_digest,
        "toolchain_manifest_sha256": tools_digest, "host_observed": tools_manifest["host_observed"],
        "controls": controls, "comparisons": reports, "artifacts": artifacts, "qualifier_host": qualifier_host,
        "pass": all(item["semantic_success"] and item["artifacts_equal"] for item in reports)}
    directories = {output}
    for name in artifact_names:
        path = output / name
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        try:
            if not stat.S_ISREG(os.fstat(fd).st_mode):
                raise ValueError("reconstruction evidence changed kind")
            os.fsync(fd)
        finally:
            os.close(fd)
        parent = path.parent
        while parent != output:
            directories.add(parent)
            parent = parent.parent
    for path in sorted(directories, key=lambda value: len(value.parts), reverse=True):
        directory = io._directory(path)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    # Publish completion last. A failed final barrier remains an error even if
    # the record is readable; incomplete output is retained and never overwritten.
    save(output, "qualification.pending", result)
    directory = io._directory(output)
    try:
        os.link("qualification.pending", "qualification.json", src_dir_fd=directory,
                dst_dir_fd=directory, follow_symlinks=False)
        os.unlink("qualification.pending", dir_fd=directory)
        os.fsync(directory)
        parent = io._directory(output.parent)
        try:
            os.fsync(parent)
        finally:
            os.close(parent)
    finally:
        os.close(directory)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    seal = commands.add_parser("seal-toolchain")
    seal.add_argument("toolchain", type=Path)
    verify = commands.add_parser("verify-toolchain")
    verify.add_argument("toolchain", type=Path)
    build = commands.add_parser("build")
    build.add_argument("source_package", type=Path)
    build.add_argument("dependencies", type=Path)
    build.add_argument("toolchain", type=Path)
    build.add_argument("output", type=Path)
    build.add_argument("--bundle", type=Path, action="append", required=True)
    args = parser.parse_args()
    try:
        if args.command == "seal-toolchain":
            manifest = seal_toolchain(args.toolchain)
            print(source.encoded({"entries": len(manifest["entries"]), "regular_bytes": manifest["regular_bytes"]}).decode().strip())
        elif args.command == "verify-toolchain":
            manifest, digest = verify_toolchain(args.toolchain)
            print(source.encoded({"manifest_sha256": digest, "entries": len(manifest["entries"])}).decode().strip())
        else:
            result = rebuild(args.source_package, args.dependencies, args.toolchain, args.bundle, args.output)
            print(source.encoded(result).decode().strip())
            raise SystemExit(0 if result["pass"] else 2)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        parser.exit(1, f"reconstruction refused: {error}\n")


if __name__ == "__main__":
    main()
