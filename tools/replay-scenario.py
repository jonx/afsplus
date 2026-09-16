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


def normalized_allocation(ranges):
    """Admit maximal logical intervals: ascending, disjoint, and never two
    adjacent intervals carrying the same unwritten flag. None declares the
    layout unmodelled by the campaign that produced it."""
    if ranges is None:
        return None
    if not isinstance(ranges, list) or len(ranges) > 4096:
        raise ValueError("expected allocation budget")
    end, flag = None, None
    for item in ranges:
        fields(item, "offset length unwritten")
        offset = integer(item["offset"], 0, MAX_FILE)
        length = integer(item["length"], 1, MAX_FILE)
        if type(item["unwritten"]) is not bool:
            raise ValueError("expected allocation kind")
        if offset % 4096 or length % 4096 or offset + length > MAX_FILE:
            raise ValueError("expected allocation block alignment")
        if end is not None and (offset < end or (offset == end and flag == item["unwritten"])):
            raise ValueError("expected allocation is not maximal")
        end, flag = offset + length, item["unwritten"]
    return ranges


BATCH_SCHEMAS = {"create": "op label parent name data", "delete": "op label",
                 "rename": "op label parent name", "replace": "op label victim parent name"}


def batch_items(operation, labels, used, deferred):
    """Admit one bounded atomic batch or one staged window namespace group.

    A window stages creates and moves only: a staged final unlink reaches the
    reserved directory at commit, which these expected-state models leave out.
    """
    items = operation["items"]
    if not isinstance(items, list) or not 1 <= len(items) <= 16:
        raise ValueError("scenario batch item count")
    payload = 0
    fresh = set()
    for item in items:
        if not isinstance(item, dict) or item.get("op") not in BATCH_SCHEMAS:
            raise ValueError("unknown scenario batch item")
        kind = item["op"]
        fields(item, BATCH_SCHEMAS[kind])
        if deferred and kind in ("delete", "replace"):
            raise ValueError("a staged window batch admits creates and moves only")
        if "parent" in item and labels.get(item["parent"]) != "directory":
            raise ValueError("unknown directory label")
        if "name" in item: name(item["name"])
        label = item["label"]
        if not isinstance(label, str) or not label.isascii() or not label.isidentifier() or len(label) > 64:
            raise ValueError("invalid scenario label")
        if kind == "create":
            if label in used: raise ValueError("scenario label reused")
            payload += data(item["data"])
            labels[label] = "file"
            used.add(label)
            fresh.add(label)
            continue
        if label in fresh or item.get("victim") in fresh:
            raise ValueError("a batch item cannot name a label created by the same batch")
        if labels.get(label) != "file":
            raise ValueError("batch item requires a file label")
        if kind == "delete":
            del labels[label]
        elif kind == "replace":
            if labels.get(item["victim"]) != "file" or item["victim"] == label:
                raise ValueError("batch replacement requires a distinct file victim")
            del labels[item["victim"]]
    return payload


def validate(encoded):
    if not isinstance(encoded, bytes) or len(encoded) > MAX_INPUT:
        raise ValueError("scenario input limit")
    scenario = json.loads(encoded, object_pairs_hook=unique)
    if not isinstance(scenario, dict):
        raise ValueError("scenario must be an object")
    version = integer(scenario.get("version"), 1, 9)
    fields(scenario, "version volume operations expected" + (" flight_capacity" if version >= 3 else "")
           + (" flight_categories flight_sink" if version >= 4 else "")
           + (" snapshot_limits expected_snapshots" if version >= 7 else "")
           + (" expected_findings" if version == 8 else "")
           + (" data_policy orphan_extents expected_orphans" if version >= 9 else ""))
    if version >= 9:
        if type(scenario["data_policy"]) is not bool:
            raise ValueError("scenario data-policy feature flag")
        integer(scenario["orphan_extents"], 1, 64)
        fields(scenario["expected_orphans"], "count bytes")
        integer(scenario["expected_orphans"]["count"], 0, MAX_OPS)
        integer(scenario["expected_orphans"]["bytes"], 0, MAX_FILE)
    if version >= 3:
        integer(scenario["flight_capacity"], 1, 256)
    if version >= 4:
        # Version 8 selects every runtime category bit, 0 through 14.
        integer(scenario["flight_categories"], 0,
                32767 if version == 8 else 127 if version >= 6 else 63 if version == 5 else 15)
        sink = scenario["flight_sink"]
        if sink is not None:
            fields(sink, "capacity disconnect_before")
            integer(sink["capacity"], 1, 256)
            if sink["disconnect_before"] is not None:
                integer(sink["disconnect_before"], 0, MAX_OPS)
    if version == 8:
        findings = scenario["expected_findings"]
        if not isinstance(findings, list) or len(findings) > MAX_OPS:
            raise ValueError("expected finding count")
        for finding in findings:
            fields(finding, "scope phase kind region ordinal object block")
            integer(finding["scope"], 1, 3)
            integer(finding["phase"], 1, 16)
            integer(finding["kind"], 1, 11)
            integer(finding["region"], 0, (1 << 32) - 1)
            for key in ("ordinal", "object", "block"):
                integer(finding[key], 0, (1 << 64) - 1)
        if findings and not (scenario["flight_categories"] & (1 << 12)
                             and scenario["flight_capacity"] == 256):
            raise ValueError("expected findings require the verify category and a full ring")
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
    if version == 8:
        # Standalone observed verification, one mount feature negotiation
        # refuses before any device write, deterministic device faults, and
        # byte edits to the image beneath the mount.
        schemas.update(verify="op", remount_refused="op", fault="op class index",
                       format_fault="op class index", corrupt="op lba offset byte",
                       reseal="op lba offset byte")
    if version >= 9:
        schemas.update(link="op label source parent name", clone_file="op label source parent name",
                       symlink="op label parent name target", set_protection="op label protection",
                       unlink_symlink="op label",
                       clone_range="op source source_offset destination destination_offset length",
                       rename_replace="op label victim parent name",
                       rename_replace_orphan="op label victim parent name",
                       orphan_file="op label", cleanup_orphan="op label",
                       preallocate="op label offset length",
                       preallocate_bounded="op label offset length max_blocks max_records",
                       set_data_policy="op label policy",
                       restore_metadata="op label protection created modified changed",
                       reclaim_step="op", snapshot_maintenance_step="op",
                       batch="op items", window_batch="op items")
    orphan_labels = set()
    for index, operation in enumerate(operations):
        if not isinstance(operation, dict) or not isinstance(operation.get("op"), str) or operation["op"] not in schemas:
            raise ValueError("unknown scenario operation")
        kind = operation["op"]
        fields(operation, schemas[kind])
        if kind in ("batch", "window_batch"):
            payload += batch_items(operation, labels, used, kind == "window_batch")
            continue
        if kind in ("fault", "format_fault"):
            classes = ("write", "flush") if kind == "format_fault" else ("write", "flush", "read")
            if operation["class"] not in classes:
                raise ValueError("scenario fault class")
            integer(operation["index"], 0, 65535)
            if kind == "format_fault" and index != 0:
                raise ValueError("a format fault must be the first operation")
            continue
        if kind in ("corrupt", "reseal"):
            integer(operation["lba"], 0, 65535)
            integer(operation["offset"], 32 if kind == "reseal" else 0, 4095)
            integer(operation["byte"], 0, 255)
            continue
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
        if kind in ("preallocate", "preallocate_bounded"):
            if integer(operation["offset"], 0, MAX_FILE) + integer(operation["length"], 0, MAX_FILE) > MAX_FILE:
                raise ValueError("scenario preallocation range limit")
            if kind == "preallocate_bounded":
                integer(operation["max_blocks"], 1, 4096)
                integer(operation["max_records"], 1, 4096)
        if kind == "set_data_policy" and type(operation["policy"]) is not bool:
            raise ValueError("scenario data-policy value")
        if kind == "restore_metadata":
            for key in ("created", "modified", "changed"):
                integer(operation[key], 0, 1 << 31)
        if kind == "rename_replace" and labels.get(operation["victim"]) != "file":
            raise ValueError("replacement requires a file victim")
        if kind == "rename_replace_orphan" and labels.get(operation["victim"]) != "file":
            raise ValueError("replacement requires a file victim")
        if kind in ("rename_replace", "rename_replace_orphan"):
            if operation["victim"] == operation["label"]:
                raise ValueError("replacement requires a distinct victim")
            del labels[operation["victim"]]
            if kind == "rename_replace_orphan":
                orphan_labels.add(operation["victim"])
        if "label" in operation:
            label = operation["label"]
            if not isinstance(label, str) or not label.isascii() or not label.isidentifier() or len(label) > 64:
                raise ValueError("invalid scenario label")
            if kind == "cleanup_orphan":
                if label not in orphan_labels:
                    raise ValueError("unknown orphan label")
                continue
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
            if kind in ("rename_replace", "rename_replace_orphan", "orphan_file", "preallocate",
                        "preallocate_bounded", "set_data_policy") and labels[label] != "file":
                raise ValueError("operation requires a file label")
            if kind == "orphan_file":
                orphan_labels.add(label)
                del labels[label]
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
    linked = {"file": "path kind links protection alias policy data alloc",
              "directory": "path kind links protection",
              "symlink": "path kind links protection target"}
    for entry in expected:
        if not isinstance(entry, dict) or entry.get("kind") not in (linked if version >= 9 else ("file", "directory")):
            raise ValueError("unknown expected kind")
        if version >= 9:
            # Linked entries carry link count, protection, alias ordinal and opaque target.
            fields(entry, linked[entry["kind"]])
            integer(entry["links"], 1, (1 << 32) - 1)
            integer(entry["protection"], 0, (1 << 32) - 1)
            if entry["kind"] == "file":
                integer(entry["alias"], 0, len(expected) - 1)
                if type(entry["policy"]) is not bool:
                    raise ValueError("expected data-policy flag")
                normalized_allocation(entry["alloc"])
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
        lines[0] = "AFSPSC09" if scenario["version"] == 9 else "AFSPSC08" if scenario["version"] == 8 else "AFSPSC07" if scenario["version"] >= 7 else "AFSPSC06" if scenario["version"] == 6 else "AFSPSC05" if scenario["version"] == 5 else "AFSPSC04"
        sink = scenario["flight_sink"]
        capacity = 0 if sink is None else sink["capacity"]
        disconnect = None if sink is None else sink["disconnect_before"]
        lines[1] += " {} {} {}".format(scenario["flight_categories"], capacity,
                                      "none" if disconnect is None else disconnect)
    if scenario["version"] >= 7:
        limits = scenario["snapshot_limits"]
        lines[1] += " {} {} {}".format(limits["max_edit_records"], limits["max_views"], limits["reclaim_records"])
    if scenario["version"] >= 9:
        lines[1] += " {} {}".format(int(scenario["data_policy"]), scenario["orphan_extents"])
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
        elif kind in ("rename_replace", "rename_replace_orphan"):
            fields = [kind, operation["label"], operation["victim"], operation["parent"],
                      operation["name"].encode().hex()]
        elif kind in ("preallocate", "preallocate_bounded"):
            fields = [kind, operation["label"], str(operation["offset"]), str(operation["length"])]
            if kind == "preallocate_bounded":
                fields += [str(operation["max_blocks"]), str(operation["max_records"])]
        elif kind == "set_data_policy":
            fields = [kind, operation["label"], str(int(operation["policy"]))]
        elif kind == "restore_metadata":
            fields = [kind, operation["label"], str(operation["protection"]), str(operation["created"]),
                      str(operation["modified"]), str(operation["changed"])]
        elif kind in ("batch", "window_batch"):
            members = []
            for item in operation["items"]:
                member = item["op"]
                if member == "delete":
                    members.append("delete:" + item["label"])
                    continue
                encoded_name = item["name"].encode().hex()
                if member == "create":
                    members.append(":".join(["create", item["label"], item["parent"], encoded_name,
                                             item["data"] or "-"]))
                elif member == "rename":
                    members.append(":".join(["rename", item["label"], item["parent"], encoded_name]))
                else:
                    members.append(":".join(["replace", item["label"], item["victim"], item["parent"],
                                             encoded_name]))
            fields = [kind, ",".join(members)]
        elif kind in ("fault", "format_fault"):
            fields = [kind, operation["class"], str(operation["index"])]
        elif kind in ("corrupt", "reseal"):
            fields = [kind, str(operation["lba"]), str(operation["offset"]), str(operation["byte"])]
        elif kind in ("unlink", "rmdir", "unlink_symlink", "orphan_file", "cleanup_orphan") or kind in (
                "snapshot_create", "snapshot_open", "snapshot_close", "snapshot_delete",
                "snapshot_inspect"):
            fields = [kind, operation["label"]]
        else:
            fields = [kind]
        lines.append(" ".join(fields))
    return ("\n".join(lines) + "\n").encode("ascii")
