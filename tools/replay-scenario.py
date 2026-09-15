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


def symlink_target(value):
    """Opaque version-9 target: admitted text, never resolved as a path."""
    if not isinstance(value, str) or value == "" or "\0" in value or len(value.encode("utf-8")) > 1024:
        raise ValueError("invalid scenario symlink target")
    return len(value.encode("utf-8"))


def captured_views(views, *, observed=False):
    """Admit explicit expected history independently of the filesystem runner."""
    if not isinstance(views, list) or len(views) > 16:
        raise ValueError("expected snapshot count")
    remaining_entries, remaining_bytes, remaining_ranges = 1024, 16 * 1024 * 1024, 4096
    previous = 0
    def metadata(value):
        fields(value, "object_id kind size allocated links protection created modified changed content_generation")
        for key in ("object_id", "size", "allocated", "content_generation"):
            integer(value[key], 1 if key == "object_id" else 0, (1 << 64) - 1)
        for key in ("links", "protection"):
            integer(value[key], 0, (1 << 32) - 1)
        if value["kind"] not in ("file", "directory", "symlink"):
            raise ValueError("expected snapshot object kind")
        for key in ("created", "modified", "changed"):
            stamp = value[key]
            if not isinstance(stamp, list) or len(stamp) != 2:
                raise ValueError("expected snapshot timestamp")
            integer(stamp[0], -(1 << 63), (1 << 63) - 1)
            integer(stamp[1], 0, 999999999)
    for view in views:
        fields(view, "id generation committed_tx_id root entries")
        previous = integer(view["id"], previous + 1, (1 << 64) - 1)
        for key in ("generation", "committed_tx_id"):
            integer(view[key], 0, (1 << 64) - 1)
        metadata(view["root"])
        if view["root"]["kind"] != "directory" or view["root"]["object_id"] != 1:
            raise ValueError("expected snapshot root")
        entries = view["entries"]
        if not isinstance(entries, list) or len(entries) > remaining_entries:
            raise ValueError("expected snapshot entries")
        remaining_entries -= len(entries)
        previous_path = None
        for entry in entries:
            fields(entry, "path metadata data allocation")
            path = entry["path"]
            if not isinstance(path, list) or not 1 <= len(path) <= 64:
                raise ValueError("expected snapshot path depth")
            for component in path: name(component)
            if previous_path is not None and path <= previous_path:
                raise ValueError("expected snapshot path ordering")
            previous_path = path
            metadata(entry["metadata"])
            if observed:
                payload = entry["data"]
                if (not isinstance(payload, str) or len(payload) % 2
                        or len(payload) > remaining_bytes * 2
                        or any(c not in "0123456789abcdef" for c in payload)):
                    raise ValueError("observed snapshot content budget or encoding")
                size = len(payload) // 2
            else:
                size = data(entry["data"])
            if size > remaining_bytes:
                raise ValueError("expected snapshot content budget")
            remaining_bytes -= size
            ranges = entry["allocation"]
            if not isinstance(ranges, list) or len(ranges) > remaining_ranges:
                raise ValueError("expected snapshot allocation budget")
            remaining_ranges -= len(ranges)
            for item in ranges:
                fields(item, "offset length unwritten")
                integer(item["offset"], 0, (1 << 64) - 1)
                integer(item["length"], 1, (1 << 64) - 1)
                if type(item["unwritten"]) is not bool:
                    raise ValueError("expected snapshot allocation kind")
            kind = entry["metadata"]["kind"]
            if kind == "directory":
                if size or ranges: raise ValueError("expected snapshot directory payload")
            elif size != entry["metadata"]["size"]:
                raise ValueError("expected snapshot content length")
            if kind == "symlink" and ranges:
                raise ValueError("expected snapshot symlink allocation")
    return views


def validate(encoded):
    if not isinstance(encoded, bytes) or len(encoded) > MAX_INPUT:
        raise ValueError("scenario input limit")
    scenario = json.loads(encoded, object_pairs_hook=unique)
    if not isinstance(scenario, dict):
        raise ValueError("scenario must be an object")
    version = integer(scenario.get("version"), 1, 9)
    if version == 8:
        raise ValueError("scenario version 8 is not admitted by this profile set")
    fields(scenario, "version volume operations expected" + (" flight_capacity" if version >= 3 else "")
           + (" flight_categories flight_sink" if version >= 4 else "")
           + (" snapshot_limits expected_snapshots" if version >= 7 else ""))
    if version >= 3:
        integer(scenario["flight_capacity"], 1, 256)
    if version >= 4:
        integer(scenario["flight_categories"], 0, 127 if version >= 6 else 63 if version == 5 else 15)
        sink = scenario["flight_sink"]
        if sink is not None:
            fields(sink, "capacity disconnect_before")
            integer(sink["capacity"], 1, 256)
            if sink["disconnect_before"] is not None:
                integer(sink["disconnect_before"], 0, MAX_OPS)
    if version >= 7:
        limits = scenario["snapshot_limits"]
        fields(limits, "max_edit_records max_views reclaim_records")
        integer(limits["max_edit_records"], 1, 4096)
        integer(limits["max_views"], 1, 16)
        integer(limits["reclaim_records"], 1, 4096)
        captured_views(scenario["expected_snapshots"])
    volume = scenario["volume"]
    fields(volume, "block_size blocks region_size log_slots" + (" tree_cache_pages" if version >= 2 else ""))
    if version >= 2:
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
    snapshot_labels = set()
    payload = 0
    schemas = {"mkdir": "op label parent name", "create": "op label parent name data",
        "write": "op label offset data", "truncate": "op label size",
        "rename": "op label parent name", "unlink": "op label", "rmdir": "op label",
        "sync": "op", "remount": "op"}
    if version >= 5:
        schemas.update(window_write="op label offset data", window_truncate="op label size",
                       window_fsync="op", window_commit="op")
    if version >= 7:
        schemas.update({"snapshot_" + action: "op label"
                        for action in ("create", "open", "close", "delete", "inspect")})
    if version >= 9:
        schemas.update(link="op label source parent name", clone_file="op label source parent name",
                       symlink="op label parent name target", set_protection="op label protection",
                       unlink_symlink="op label",
                       clone_range="op source source_offset destination destination_offset length")
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
        for key in ("source", "destination"):
            if key in operation and (not isinstance(operation[key], str) or labels.get(operation[key]) != "file"):
                raise ValueError("operation requires a file label")
        if kind == "clone_range":
            length = integer(operation["length"], 0, MAX_FILE)
            for key in ("source_offset", "destination_offset"):
                if integer(operation[key], 0, MAX_FILE) + length > MAX_FILE:
                    raise ValueError("scenario clone range limit")
        if "target" in operation: payload += symlink_target(operation["target"])
        if "protection" in operation: integer(operation["protection"], 0, (1 << 32) - 1)
        if "label" in operation:
            label = operation["label"]
            if not isinstance(label, str) or not label.isascii() or not label.isidentifier() or len(label) > 64:
                raise ValueError("invalid scenario label")
            if kind.startswith("snapshot_"):
                if kind == "snapshot_create":
                    if label in snapshot_labels: raise ValueError("snapshot label reused")
                    snapshot_labels.add(label)
                elif label not in snapshot_labels:
                    raise ValueError("unknown snapshot label")
                continue
            if kind in ("mkdir", "create", "link", "symlink", "clone_file"):
                if label in used: raise ValueError("scenario label reused")
                labels[label] = {"mkdir": "directory", "symlink": "symlink"}.get(kind, "file")
                used.add(label)
            elif label not in labels or label == "root":
                raise ValueError("unknown or reserved object label")
            if kind in ("write", "truncate", "window_write", "window_truncate", "unlink") and labels[label] != "file":
                raise ValueError("operation requires a file label")
            if kind == "rmdir" and labels[label] != "directory":
                raise ValueError("operation requires a directory label")
            if kind == "unlink_symlink" and labels[label] != "symlink":
                raise ValueError("operation requires a symlink label")
            if kind in ("unlink", "rmdir", "unlink_symlink"): del labels[label]
        if "data" in operation:
            count = data(operation["data"])
            payload += count
            if kind in ("write", "window_write") and integer(operation["offset"], 0, MAX_FILE) + count > MAX_FILE:
                raise ValueError("scenario write range limit")
        if "size" in operation: integer(operation["size"], 0, MAX_FILE)
    expected = scenario["expected"]
    if not isinstance(expected, list) or len(expected) > MAX_OPS:
        raise ValueError("expected-state limit")
    paths = set()
    linked = {"file": "path kind links protection alias data", "directory": "path kind links protection",
              "symlink": "path kind links protection target"}
    for entry in expected:
        if not isinstance(entry, dict) or entry.get("kind") not in (linked if version >= 9 else ("file", "directory")):
            raise ValueError("unknown expected kind")
        if version >= 9:
            # Linked entries carry link count, protection, alias ordinal and opaque target.
            fields(entry, linked[entry["kind"]])
            integer(entry["links"], 1, (1 << 32) - 1)
            integer(entry["protection"], 0, (1 << 32) - 1)
            if entry["kind"] == "file": integer(entry["alias"], 0, len(expected) - 1)
            if entry["kind"] == "symlink": payload += symlink_target(entry["target"])
        else:
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
    if scenario["version"] >= 2:
        lines[0] = "AFSPSC02"
        lines[1] += " " + str(volume["tree_cache_pages"])
    if scenario["version"] >= 3:
        lines[0] = "AFSPSC03"
        lines[1] += " " + str(scenario["flight_capacity"])
    if scenario["version"] >= 4:
        lines[0] = "AFSPSC09" if scenario["version"] == 9 else "AFSPSC07" if scenario["version"] >= 7 else "AFSPSC06" if scenario["version"] == 6 else "AFSPSC05" if scenario["version"] == 5 else "AFSPSC04"
        sink = scenario["flight_sink"]
        capacity = 0 if sink is None else sink["capacity"]
        disconnect = None if sink is None else sink["disconnect_before"]
        lines[1] += " {} {} {}".format(scenario["flight_categories"], capacity,
                                      "none" if disconnect is None else disconnect)
    if scenario["version"] >= 7:
        limits = scenario["snapshot_limits"]
        lines[1] += " {} {} {}".format(limits["max_edit_records"], limits["max_views"], limits["reclaim_records"])
    for operation in scenario["operations"]:
        kind = operation["op"]
        if kind in ("mkdir", "create"):
            fields = [kind, operation["label"], operation["parent"], operation["name"].encode().hex()]
            if kind == "create": fields.append(operation["data"] or "-")
        elif kind in ("write", "window_write"):
            fields = [kind, operation["label"], str(operation["offset"]), operation["data"] or "-"]
        elif kind in ("truncate", "window_truncate"):
            fields = [kind, operation["label"], str(operation["size"])]
        elif kind == "rename":
            fields = [kind, operation["label"], operation["parent"], operation["name"].encode().hex()]
        elif kind in ("link", "clone_file"):
            fields = [kind, operation["label"], operation["source"], operation["parent"],
                      operation["name"].encode().hex()]
        elif kind == "symlink":
            fields = [kind, operation["label"], operation["parent"], operation["name"].encode().hex(),
                      operation["target"].encode().hex()]
        elif kind == "clone_range":
            fields = [kind, operation["source"], str(operation["source_offset"]), operation["destination"],
                      str(operation["destination_offset"]), str(operation["length"])]
        elif kind == "set_protection":
            fields = [kind, operation["label"], str(operation["protection"])]
        elif kind in ("unlink", "rmdir", "unlink_symlink") or kind.startswith("snapshot_"):
            fields = [kind, operation["label"]]
        else:
            fields = [kind]
        lines.append(" ".join(fields))
    return ("\n".join(lines) + "\n").encode("ascii")
