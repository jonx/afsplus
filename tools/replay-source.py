#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""Preserve and restore a bounded private Git revision, index and working source."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import selectors
import signal
import stat
import subprocess
import tempfile
import time

spec = importlib.util.spec_from_file_location("source_bundle_io", Path(__file__).with_name("replay-bundle.py"))
io = importlib.util.module_from_spec(spec)
spec.loader.exec_module(io)
FILE_BYTES = 64 * 1024 * 1024
TOTAL_BYTES = 256 * 1024 * 1024
MANIFEST_BYTES = 8 * 1024 * 1024
MAX_FILES = 32768


def encoded(value):
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


def sha(data):
    return hashlib.sha256(data).hexdigest()


def git(root, *args, data=None, limit=MANIFEST_BYTES, isolated=False):
    """Bound command output and time; never invoke a shell or supplied command."""
    env = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
    env["GIT_TERMINAL_PROMPT"] = "0"
    if isolated:
        env.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull)
    command = ["git", "-c", "core.hooksPath=" + os.devnull, "-c", "core.fsmonitor=false",
               "-c", "protocol.allow=never", "-c", "protocol.file.allow=always", *args]
    with tempfile.TemporaryFile() as inputs:
        if data is not None:
            inputs.write(data)
            inputs.seek(0)
        with subprocess.Popen(command, cwd=root, env=env, stdin=inputs, stdout=subprocess.PIPE,
                              stderr=subprocess.PIPE, start_new_session=True) as process:
            chunks = []
            size = 0
            error_chunks = []
            error_size = 0
            deadline = time.monotonic() + 120
            try:
                with selectors.DefaultSelector() as selector:
                    selector.register(process.stdout, selectors.EVENT_READ, "output")
                    selector.register(process.stderr, selectors.EVENT_READ, "errors")
                    while selector.get_map():
                        if time.monotonic() >= deadline:
                            raise ValueError("Git operation timed out")
                        ready = selector.select(min(1, max(0, deadline - time.monotonic())))
                        for key, _ in ready:
                            chunk = os.read(key.fileobj.fileno(), 65536)
                            if not chunk:
                                selector.unregister(key.fileobj)
                                continue
                            if key.data == "output":
                                size += len(chunk)
                                if size > limit:
                                    raise ValueError("Git output exceeds source-package bound")
                                chunks.append(chunk)
                            else:
                                error_size += len(chunk)
                                if error_size > 65536:
                                    raise ValueError("Git diagnostics exceed source-package bound")
                                error_chunks.append(chunk)
                result = process.wait(timeout=max(0.01, deadline - time.monotonic()))
                if result:
                    raise ValueError("Git operation failed: " + b"".join(error_chunks)[:4096].decode(errors="replace"))
                return b"".join(chunks)
            except BaseException:
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                except PermissionError:
                    if process.poll() is None:
                        process.kill()
                process.wait()
                raise


def path_bytes(value):
    if not isinstance(value, str):
        raise ValueError("source path encoding")
    try:
        raw = bytes.fromhex(value)
    except ValueError as error:
        raise ValueError("source path encoding") from error
    if raw.hex() != value or not raw or len(raw) > 4096 or b"\0" in raw or b"\\" in raw:
        raise ValueError("source path encoding")
    parts = raw.split(b"/")
    if any(part in (b"", b".", b"..") or part.lower().rstrip(b" .") == b".git" for part in parts):
        raise ValueError("unsafe source path")
    return raw


def index_entries(data):
    if data and not data.endswith(b"\0"):
        raise ValueError("truncated source index")
    records = []
    seen = set()
    for raw in data.split(b"\0")[:-1]:
        try:
            prefix, path = raw.split(b"\t", 1)
            mode, oid, stage = prefix.decode("ascii").split(" ")
        except (ValueError, UnicodeError) as error:
            raise ValueError("source index record") from error
        path_bytes(path.hex())
        if mode not in ("100644", "100755", "120000") or stage not in ("0", "1", "2", "3"):
            raise ValueError("unsupported source index mode or stage")
        if not isinstance(oid, str) or len(oid) not in (40, 64) or any(c not in "0123456789abcdef" for c in oid):
            raise ValueError("source index object identity")
        if (path, stage) in seen:
            raise ValueError("duplicate source index entry")
        seen.add((path, stage))
        records.append((path, oid))
    if len(records) > MAX_FILES * 3:
        raise ValueError("source index entry bound")
    return records


def safe_link(path, target):
    if not target or len(target) > 4096 or b"\0" in target or target.startswith(b"/") or b"\\" in target:
        raise ValueError("unsafe source symlink")
    parts = path.split(b"/")[:-1]
    for part in target.split(b"/"):
        if part in (b"", b"."):
            continue
        if part == b"..":
            if not parts:
                raise ValueError("source symlink escapes checkout")
            parts.pop()
        else:
            if part.lower().rstrip(b" .") == b".git":
                raise ValueError("source symlink references Git internals")
            parts.append(part)


def read_source(root, path):
    """Walk parents without following links, including before final lstat."""
    directory = io._directory(root)
    try:
        parts = os.fsdecode(path).split("/")
        for part in parts[:-1]:
            try:
                child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=directory)
            except FileNotFoundError:
                return {"path": path.hex(), "kind": "missing"}, None
            os.close(directory)
            directory = child
        try:
            info = os.stat(parts[-1], dir_fd=directory, follow_symlinks=False)
        except FileNotFoundError:
            return {"path": path.hex(), "kind": "missing"}, None
        mode = stat.S_IMODE(info.st_mode)
        if mode & ~0o777:
            raise ValueError("unsupported special source mode bits")
        if stat.S_ISLNK(info.st_mode):
            data = os.fsencode(os.readlink(parts[-1], dir_fd=directory))
            safe_link(path, data)
            kind = "link"
        elif stat.S_ISREG(info.st_mode):
            data = io._read(directory, parts[-1], FILE_BYTES)
            kind = "file"
        else:
            raise ValueError("unsupported source file kind")
        return {"path": path.hex(), "kind": kind, "mode": mode, "sha256": sha(data)}, data
    finally:
        os.close(directory)


def scan(root):
    raw = git(root, "ls-files", "-z", "--cached", "--others", "--exclude-standard")
    paths = sorted(set(raw.split(b"\0")) - {b""})
    if len(paths) > MAX_FILES:
        raise ValueError("source file count bound")
    entries, blobs = [], {}
    size = 0
    for path in paths:
        path_bytes(path.hex())
        entry, data = read_source(root, path)
        entries.append(entry)
        if data is not None:
            size += len(data)
            blobs[sha(data)] = data
        if size > TOTAL_BYTES:
            raise ValueError("expanded source worktree bound")
    return entries, blobs


def identity(revision, entries):
    tree = hashlib.sha256()
    for entry in entries:
        path = path_bytes(entry["path"])
        tree.update(len(path).to_bytes(4, "little") + path)
        if entry["kind"] == "missing":
            tree.update(b"missing")
        else:
            tree.update(str(entry["mode"]).encode() + b"\0")
            tree.update(entry["kind"].encode() + bytes.fromhex(entry["sha256"]))
    return {"revision": revision, "working_tree_sha256": tree.hexdigest()}


def fresh_destination(output, protected):
    output = Path(output)
    resolved = output.resolve()
    for source in protected:
        source = Path(source).resolve()
        if resolved == source or source in resolved.parents:
            raise ValueError("source output overlaps protected input")
    if os.path.lexists(output):
        raise ValueError("source output already exists")
    return output


def capture(root, output):
    root = Path(root).resolve()
    output = fresh_destination(output, [root])
    if Path(os.fsdecode(git(root, "rev-parse", "--show-toplevel").removesuffix(b"\n"))).resolve() != root:
        raise ValueError("capture requires the repository root")
    revision = git(root, "rev-parse", "HEAD").decode().strip()
    index = git(root, "ls-files", "--stage", "-z")
    staged = index_entries(index)
    entries, blobs = scan(root)
    objects = []
    for oid in sorted({oid for _, oid in staged}):
        data = git(root, "cat-file", "blob", oid, limit=FILE_BYTES)
        blobs[sha(data)] = data
        objects.append({"oid": oid, "sha256": sha(data)})
        if sum(map(len, blobs.values())) > TOTAL_BYTES:
            raise ValueError("source aggregate bound")
    history = git(root, "bundle", "create", "-", "HEAD", limit=FILE_BYTES)
    blobs[sha(history)] = history
    blobs[sha(index)] = index
    after, _ = scan(root)
    if (after != entries or git(root, "rev-parse", "HEAD").decode().strip() != revision
            or git(root, "ls-files", "--stage", "-z") != index):
        raise ValueError("source changed during capture")
    manifest = {"version": 1, "kind": "git-working-source-v1", "source_observed": identity(revision, entries),
        "files": entries, "history_sha256": sha(history), "index_sha256": sha(index), "git_objects": objects,
        "artifacts": [{"sha256": key, "size": len(data)} for key, data in sorted(blobs.items())]}
    admit(manifest, blobs)
    output.mkdir(mode=0o700)
    directory = io._directory(output)
    try:
        for key, data in sorted(blobs.items()):
            io._write(directory, "blob-" + key, data)
        os.fsync(directory)
        io._write(directory, "manifest.pending", encoded(manifest))
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
    return manifest["source_observed"]


def descriptors(manifest):
    fields = {"version", "kind", "source_observed", "files", "history_sha256", "index_sha256", "git_objects", "artifacts"}
    if not isinstance(manifest, dict) or set(manifest) != fields or type(manifest["version"]) is not int or manifest["version"] != 1 or manifest["kind"] != "git-working-source-v1":
        raise ValueError("source manifest fields or version")
    if len(encoded(manifest)) > MANIFEST_BYTES:
        raise ValueError("source manifest size bound")
    artifacts = manifest["artifacts"]
    if not isinstance(artifacts, list) or len(artifacts) > MAX_FILES * 4 + 2:
        raise ValueError("source artifact count")
    result = {}
    for item in artifacts:
        if not isinstance(item, dict) or set(item) != {"sha256", "size"}:
            raise ValueError("source artifact descriptor")
        key, size = item["sha256"], item["size"]
        if not isinstance(key, str) or len(key) != 64 or any(c not in "0123456789abcdef" for c in key) or key in result:
            raise ValueError("source artifact digest")
        if type(size) is not int or not 0 <= size <= FILE_BYTES:
            raise ValueError("source artifact size")
        result[key] = size
    if sum(result.values()) > TOTAL_BYTES:
        raise ValueError("source aggregate bound")
    return result


def admit(manifest, blobs):
    expected = descriptors(manifest)
    if set(blobs) != set(expected) or any(len(blobs[key]) != size or sha(blobs[key]) != key for key, size in expected.items()):
        raise ValueError("source artifact integrity")
    observed = manifest["source_observed"]
    if not isinstance(observed, dict) or set(observed) != {"revision", "working_tree_sha256"}:
        raise ValueError("source observed identity")
    revision = observed["revision"]
    if not isinstance(revision, str) or len(revision) not in (40, 64) or any(c not in "0123456789abcdef" for c in revision):
        raise ValueError("source revision identity")
    entries = manifest["files"]
    if not isinstance(entries, list) or len(entries) > MAX_FILES:
        raise ValueError("source file count")
    paths = set()
    used = set()
    expanded = 0
    for entry in entries:
        if not isinstance(entry, dict):
            raise ValueError("source file entry")
        if entry.get("kind") == "missing":
            fields = {"path", "kind"}
        elif entry.get("kind") in ("file", "link"):
            fields = {"path", "kind", "mode", "sha256"}
            if type(entry.get("mode")) is not int or not 0 <= entry["mode"] <= 0o777:
                raise ValueError("source file mode")
            key = entry.get("sha256")
            if not isinstance(key, str) or key not in blobs:
                raise ValueError("source file blob")
            used.add(key)
            expanded += len(blobs[key])
            if expanded > TOTAL_BYTES:
                raise ValueError("expanded source worktree bound")
        else:
            raise ValueError("source file kind")
        if set(entry) != fields:
            raise ValueError("source file fields")
        path = path_bytes(entry["path"])
        if path in paths:
            raise ValueError("duplicate source path")
        paths.add(path)
        if entry["kind"] == "link":
            safe_link(path, blobs[entry["sha256"]])
    if [path_bytes(entry["path"]) for entry in entries] != sorted(paths):
        raise ValueError("source paths are not ordered")
    for path in paths:
        parts = path.split(b"/")
        if any(b"/".join(parts[:n]) in paths for n in range(1, len(parts))):
            raise ValueError("source path has a file ancestor")
    if identity(revision, entries) != observed:
        raise ValueError("source tree identity mismatch")
    for role in ("history_sha256", "index_sha256"):
        key = manifest[role]
        if not isinstance(key, str) or key not in blobs:
            raise ValueError("source history or index blob")
        used.add(key)
    index = index_entries(blobs[manifest["index_sha256"]])
    if not {path for path, _ in index} <= paths:
        raise ValueError("source index path missing from inventory")
    objects = manifest["git_objects"]
    if not isinstance(objects, list) or len(objects) > MAX_FILES * 3:
        raise ValueError("source index object count")
    oids = set()
    for item in objects:
        if not isinstance(item, dict) or set(item) != {"oid", "sha256"} or not isinstance(item["oid"], str) or item["oid"] in oids or not isinstance(item["sha256"], str) or item["sha256"] not in blobs:
            raise ValueError("source index object descriptor")
        oids.add(item["oid"])
        data = blobs[item["sha256"]]
        algorithm = hashlib.sha1 if len(item["oid"]) == 40 else hashlib.sha256
        if algorithm(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest() != item["oid"]:
            raise ValueError("source index blob identity mismatch")
        used.add(item["sha256"])
    if oids != {oid for _, oid in index} or used != set(blobs):
        raise ValueError("source object coverage")


def read_package(path):
    directory = io._directory(path)
    try:
        manifest = json.loads(io._read(directory, "manifest.json", MANIFEST_BYTES), object_pairs_hook=io._unique)
        expected = descriptors(manifest)
        blobs = {key: io._read(directory, "blob-" + key, size) for key, size in expected.items()}
    finally:
        os.close(directory)
    admit(manifest, blobs)
    return manifest, blobs


def restore(package, output):
    output = fresh_destination(output, [package])
    manifest, blobs = read_package(package)
    # Git gets a captured regular bundle, never a metadata-selected URL or path.
    with tempfile.TemporaryDirectory(prefix="afsplus-source-history-") as temporary:
        history = Path(temporary) / "history.bundle"
        history.write_bytes(blobs[manifest["history_sha256"]])
        output.mkdir(mode=0o700)
        git(None, "clone", "--quiet", "--no-checkout", "--template=", "--", str(history), str(output), isolated=True)
    if git(output, "rev-parse", "HEAD", isolated=True).decode().strip() != manifest["source_observed"]["revision"]:
        raise ValueError("source history revision mismatch")
    git(output, "config", "--local", "core.excludesFile", os.devnull, isolated=True)
    git(output, "read-tree", "--empty", isolated=True)
    for item in manifest["git_objects"]:
        actual = git(output, "hash-object", "-w", "--stdin", data=blobs[item["sha256"]], isolated=True).decode().strip()
        if actual != item["oid"]:
            raise ValueError("source index blob identity mismatch")
    git(output, "update-index", "-z", "--index-info", data=blobs[manifest["index_sha256"]], isolated=True)
    for entry in manifest["files"]:
        if entry["kind"] == "missing":
            continue
        path = output / os.fsdecode(path_bytes(entry["path"]))
        path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        data = blobs[entry["sha256"]]
        if entry["kind"] == "link":
            os.symlink(os.fsdecode(data), path)
            if stat.S_IMODE(path.lstat().st_mode) != entry["mode"]:
                try:
                    os.chmod(path, entry["mode"], follow_symlinks=False)
                except NotImplementedError as error:
                    raise ValueError("host cannot restore source symlink mode") from error
        else:
            fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
            with os.fdopen(fd, "wb") as stream:
                stream.write(data)
                os.fchmod(stream.fileno(), entry["mode"])
                stream.flush()
                os.fsync(stream.fileno())
    for entry in manifest["files"]:
        if entry["kind"] == "link":
            try:
                target = (output / os.fsdecode(path_bytes(entry["path"]))).resolve()
            except RuntimeError as error:
                raise ValueError("source symlink cycle") from error
            if output.resolve() != target and output.resolve() not in target.parents:
                raise ValueError("resolved source symlink escapes checkout")
    entries, _ = scan(output)
    if (identity(manifest["source_observed"]["revision"], entries) != manifest["source_observed"]
            or git(output, "ls-files", "--stage", "-z") != blobs[manifest["index_sha256"]]):
        raise ValueError("restored source identity mismatch")
    return manifest["source_observed"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    for command in ("capture", "restore"):
        item = commands.add_parser(command)
        item.add_argument("input", type=Path)
        item.add_argument("output", type=Path)
    args = parser.parse_args()
    try:
        result = (capture if args.command == "capture" else restore)(args.input, args.output)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        parser.exit(1, f"source package refused: {error}\n")
    print(encoded(result).decode().strip())


if __name__ == "__main__":
    main()
