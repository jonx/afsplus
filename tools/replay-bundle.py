#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""Bounded publication and integrity verification for private replay bundles."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import stat

ROLES = frozenset(("run.json", "operations.afstrace", "block-io.afstrace",
    "flight-recorder.bin", "start.img", "result.img", "fault-model.json",
    "expected.json", "actual.json"))
MANIFEST = "manifest.json"
DEFAULT_FILE_BYTES = 64 * 1024 * 1024
DEFAULT_TOTAL_BYTES = 256 * 1024 * 1024


def _directory(path):
    return os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)


def _write(directory, name, data):
    fd = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=directory)
    with os.fdopen(fd, "wb") as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())


def publish(path, artifacts, file_bytes=DEFAULT_FILE_BYTES, total_bytes=DEFAULT_TOTAL_BYTES):
    """Preserve failed exports for diagnosis; never overwrite existing paths."""
    if set(artifacts) != ROLES or file_bytes < 0 or total_bytes < 0:
        raise ValueError("invalid bundle roles or limits")
    if any(not isinstance(data, bytes) or len(data) > file_bytes for data in artifacts.values()):
        raise ValueError("artifact type or size limit")
    if sum(map(len, artifacts.values())) > total_bytes:
        raise ValueError("bundle total size limit")
    manifest = {"schema_version": 1, "artifacts": [
        {"name": name, "size": len(artifacts[name]), "sha256": hashlib.sha256(artifacts[name]).hexdigest()}
        for name in sorted(ROLES)]}
    encoded = (json.dumps(manifest, sort_keys=True, separators=(",", ":")) + "\n").encode()
    path = Path(path)
    parent = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.mkdir(path.name, 0o700, dir_fd=parent)
        directory = os.open(path.name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=parent)
        try:
            for name in sorted(ROLES):
                _write(directory, name, artifacts[name])
            # Persist artifact names before publishing the completion manifest.
            os.fsync(directory)
            _write(directory, "manifest.pending", encoded)
            os.link("manifest.pending", MANIFEST, src_dir_fd=directory, dst_dir_fd=directory, follow_symlinks=False)
            os.unlink("manifest.pending", dir_fd=directory)
            os.fsync(directory)
            # The newly created bundle name also needs its parent's barrier.
            os.fsync(parent)
        finally:
            os.close(directory)
    finally:
        os.close(parent)


def _unique(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate manifest key")
        result[key] = value
    return result


def _read(directory, name, limit):
    fd = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=directory)
    with os.fdopen(fd, "rb") as stream:
        info = os.fstat(stream.fileno())
        if not stat.S_ISREG(info.st_mode) or info.st_size > limit:
            raise ValueError("artifact must be a bounded regular file")
        data = stream.read(limit + 1)
        if len(data) > limit:
            raise ValueError("artifact grew past limit")
        return data


def read_bundle(path, file_bytes=DEFAULT_FILE_BYTES, total_bytes=DEFAULT_TOTAL_BYTES):
    """Return captured bytes only after validating every role and digest.

    This verifies integrity/completeness, not semantic scenario success.
    """
    if file_bytes < 0 or total_bytes < 0:
        raise ValueError("negative bundle limit")
    directory = _directory(path)
    try:
        manifest = json.loads(_read(directory, MANIFEST, 65536), object_pairs_hook=_unique)
        if not isinstance(manifest, dict) or set(manifest) != {"schema_version", "artifacts"}:
            raise ValueError("invalid manifest fields")
        if type(manifest["schema_version"]) is not int or manifest["schema_version"] != 1:
            raise ValueError("unknown manifest version")
        entries = manifest["artifacts"]
        if not isinstance(entries, list) or len(entries) != len(ROLES):
            raise ValueError("missing artifact roles")
        seen = set()
        size = 0
        for entry in entries:
            if not isinstance(entry, dict) or set(entry) != {"name", "size", "sha256"}:
                raise ValueError("invalid artifact descriptor")
            name = entry["name"]
            if not isinstance(name, str) or name not in ROLES or name in seen:
                raise ValueError("unknown or duplicate artifact role")
            seen.add(name)
            if type(entry["size"]) is not int or not 0 <= entry["size"] <= file_bytes:
                raise ValueError("artifact size limit")
            digest = entry["sha256"]
            if not isinstance(digest, str) or len(digest) != 64 or any(c not in "0123456789abcdef" for c in digest):
                raise ValueError("invalid artifact digest")
            size += entry["size"]
            if size > total_bytes:
                raise ValueError("bundle total size limit")
        captured = {}
        for entry in entries:
            data = _read(directory, entry["name"], entry["size"])
            if len(data) != entry["size"] or hashlib.sha256(data).hexdigest() != entry["sha256"]:
                raise ValueError("artifact integrity mismatch")
            captured[entry["name"]] = data
        return captured
    finally:
        os.close(directory)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--file-bytes", type=int, default=DEFAULT_FILE_BYTES)
    parser.add_argument("--total-bytes", type=int, default=DEFAULT_TOTAL_BYTES)
    args = parser.parse_args()
    try:
        artifacts = read_bundle(args.directory, args.file_bytes, args.total_bytes)
    except (OSError, ValueError) as error:
        parser.exit(1, f"bundle verification failed: {error}\n")
    print(f"bundle integrity verified: {len(artifacts)} artifacts; semantic result not evaluated")


if __name__ == "__main__":
    main()
