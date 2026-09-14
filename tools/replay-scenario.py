#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""Strict bounded semantic scenario admission for the Stage A replay runner."""
import json

MAX_INPUT = 4 * 1024 * 1024
MAX_OPS = 1024
MAX_DATA = 1024 * 1024
MAX_FILE = 16 * 1024 * 1024


def unique(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate scenario key")
        result[key] = value
    return result


def fields(value, names):
    if not isinstance(value, dict) or set(value) != set(names.split()):
        raise ValueError("unknown or missing scenario fields")


def integer(value, minimum, maximum):
    if type(value) is not int or not minimum <= value <= maximum:
        raise ValueError("scenario integer outside admission")
    return value


def name(value):
    if not isinstance(value, str) or value == "" or "/" in value or "\0" in value:
        raise ValueError("invalid scenario name component")
    if len(value.encode("utf-8")) > 255:
        raise ValueError("scenario name byte limit")
    return value


def data(value):
    if not isinstance(value, str) or len(value) % 2 or len(value) > MAX_DATA * 2:
        raise ValueError("scenario payload limit")
    if any(c not in "0123456789abcdef" for c in value):
        raise ValueError("payload must be canonical lowercase hexadecimal")
    return len(value) // 2


def validate(encoded):
    if not isinstance(encoded, bytes) or len(encoded) > MAX_INPUT:
        raise ValueError("scenario input limit")
    scenario = json.loads(encoded, object_pairs_hook=unique)
    fields(scenario, "version volume operations expected")
    version = integer(scenario["version"], 1, 2)
    volume = scenario["volume"]
    fields(volume, "block_size blocks region_size log_slots" + (" tree_cache_pages" if version == 2 else ""))
    if version == 2:
        pages = volume["tree_cache_pages"]
        if not ((type(pages) is int and pages in (2, 4, 8)) or pages == "unlimited"):
            raise ValueError("unsupported scenario tree cache profile")
    # This runner profile follows mkfs_impl; larger format profiles need qualification.
    bs = integer(volume["block_size"], 4096, 4096)
    region = integer(volume["region_size"], 64, 16384)
    if bs & (bs - 1) or region & (region - 1):
        raise ValueError("scenario geometry must use powers of two")
    blocks = integer(volume["blocks"], 64, 65536)
    if blocks * bs > 256 * 1024 * 1024 or region > blocks:
        raise ValueError("scenario image limit")
    integer(volume["log_slots"], 8, 64)
    operations = scenario["operations"]
    if not isinstance(operations, list) or len(operations) > MAX_OPS:
        raise ValueError("scenario operation limit")
    labels = {"root": "directory"}
    used = {"root"}
    payload = 0
    schemas = {"mkdir": "op label parent name", "create": "op label parent name data",
        "write": "op label offset data", "truncate": "op label size",
        "rename": "op label parent name", "unlink": "op label", "rmdir": "op label",
        "sync": "op", "remount": "op"}
    for operation in operations:
        if not isinstance(operation, dict) or not isinstance(operation.get("op"), str) or operation["op"] not in schemas:
            raise ValueError("unknown scenario operation")
        kind = operation["op"]
        fields(operation, schemas[kind])
        if "name" in operation: name(operation["name"])
        if "parent" in operation:
            parent = operation["parent"]
            if not isinstance(parent, str) or labels.get(parent) != "directory":
                raise ValueError("unknown directory label")
        if "label" in operation:
            label = operation["label"]
            if not isinstance(label, str) or not label.isascii() or not label.isidentifier() or len(label) > 64:
                raise ValueError("invalid scenario label")
            if kind in ("mkdir", "create"):
                if label in used: raise ValueError("scenario label reused")
                labels[label] = "directory" if kind == "mkdir" else "file"
                used.add(label)
            elif label not in labels or label == "root":
                raise ValueError("unknown or reserved object label")
            if kind in ("write", "truncate", "unlink") and labels[label] != "file":
                raise ValueError("operation requires a file label")
            if kind == "rmdir" and labels[label] != "directory":
                raise ValueError("operation requires a directory label")
            if kind in ("unlink", "rmdir"): del labels[label]
        if "data" in operation:
            count = data(operation["data"])
            payload += count
            if kind == "write" and integer(operation["offset"], 0, MAX_FILE) + count > MAX_FILE:
                raise ValueError("scenario write range limit")
        if "size" in operation: integer(operation["size"], 0, MAX_FILE)
    expected = scenario["expected"]
    if not isinstance(expected, list) or len(expected) > MAX_OPS:
        raise ValueError("expected-state limit")
    paths = set()
    for entry in expected:
        if not isinstance(entry, dict) or entry.get("kind") not in ("file", "directory"):
            raise ValueError("unknown expected kind")
        fields(entry, "path kind data" if entry["kind"] == "file" else "path kind")
        path = entry["path"]
        if not isinstance(path, list) or not 1 <= len(path) <= 64:
            raise ValueError("expected path depth limit")
        components = tuple(name(component) for component in path)
        if components in paths: raise ValueError("duplicate expected path")
        paths.add(components)
        if entry["kind"] == "file": payload += data(entry["data"])
    if payload > MAX_DATA:
        raise ValueError("aggregate scenario payload limit")
    return scenario


def compile_commands(encoded):
    """Compile admitted JSON to the runner's fixed ASCII operation protocol.

    Names/data are hex fields; no field can become a host path or shell command.
    Expected state stays in the enclosing scenario for independent comparison.
    """
    scenario = validate(encoded)
    volume = scenario["volume"]
    lines = ["AFSPSC01", "format {} {} {} {}".format(
        volume["block_size"], volume["blocks"], volume["region_size"], volume["log_slots"])]
    if scenario["version"] == 2:
        lines[0] = "AFSPSC02"
        lines[1] += " " + str(volume["tree_cache_pages"])
    for operation in scenario["operations"]:
        kind = operation["op"]
        if kind in ("mkdir", "create"):
            fields = [kind, operation["label"], operation["parent"], operation["name"].encode().hex()]
            if kind == "create": fields.append(operation["data"] or "-")
        elif kind == "write":
            fields = [kind, operation["label"], str(operation["offset"]), operation["data"] or "-"]
        elif kind == "truncate":
            fields = [kind, operation["label"], str(operation["size"])]
        elif kind == "rename":
            fields = [kind, operation["label"], operation["parent"], operation["name"].encode().hex()]
        elif kind in ("unlink", "rmdir"):
            fields = [kind, operation["label"]]
        else:
            fields = [kind]
        lines.append(" ".join(fields))
    return ("\n".join(lines) + "\n").encode("ascii")
