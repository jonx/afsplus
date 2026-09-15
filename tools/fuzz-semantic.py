#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""Seeded namespace/content and operation-family properties through the bounded semantic replay runner."""
import argparse
import importlib.util
import json
import os
import subprocess
import sys
from pathlib import Path

spec = importlib.util.spec_from_file_location("semantic_properties_runner", Path(__file__).with_name("afsptest.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)
VERSION = 1
EXTENDED_VERSION = 2
MASK = (1 << 64) - 1
PROFILES = (2, 4, 8, "unlimited")
BLOCK = 4096
MAX_FILE_BYTES = 8224
# Scenario JSON version selected by each generated operation family.
FAMILY_VERSIONS = {"window": 5, "snapshot": 7, "namespace": 9}
CONTROLS = {"window": ("window-byte",), "snapshot": ("snapshot-entry",),
            "namespace": ("link-count", "symlink-target", "protection", "clone-byte", "directory-rename")}
SNAPSHOT_LIMITS = {"max_edit_records": 4096, "max_views": 16, "reclaim_records": 8}
SYMLINK_TARGETS = ("a", "../up", "/absolute/path", "ünïcode/ταξί", "x" * 200, "loop/../loop")
PROTECTIONS = (0, 1, 5, 0o755, 0xFFFFFFFF)


class Random:
    """Fixed xorshift64 sequence; no dependency on Python's random version."""
    def __init__(self, seed):
        self.state = seed or 0x9E3779B97F4A7C15

    def next(self):
        value = self.state
        value ^= (value << 13) & MASK
        value ^= value >> 7
        value ^= (value << 17) & MASK
        self.state = value & MASK
        return self.state

    def pick(self, values):
        return values[self.next() % len(values)]


class Model:
    """Independent flat object graph and byte arrays, with no filesystem codecs."""
    def __init__(self):
        self.nodes = {"root": {"kind": "directory", "parent": None, "name": ""}}

    def apply(self, operation):
        kind = operation["op"]
        if kind in ("sync", "remount"):
            return
        label = operation["label"]
        if kind in ("create", "mkdir"):
            if label in self.nodes or self.nodes[operation["parent"]]["kind"] != "directory":
                raise ValueError("invalid model create")
            self._vacant(operation["parent"], operation["name"])
            node = {"kind": "file" if kind == "create" else "directory",
                    "parent": operation["parent"], "name": operation["name"]}
            if kind == "create":
                node["data"] = bytearray.fromhex(operation["data"])
            self.nodes[label] = node
        elif kind == "write":
            data = self.nodes[label]["data"]
            incoming = bytes.fromhex(operation["data"])
            if incoming:
                end = operation["offset"] + len(incoming)
                data.extend(b"\0" * max(0, end - len(data)))
                data[operation["offset"]:end] = incoming
        elif kind == "truncate":
            data = self.nodes[label]["data"]
            size = operation["size"]
            del data[size:]
            data.extend(b"\0" * max(0, size - len(data)))
        elif kind == "rename":
            node = self.nodes[label]
            if node["kind"] != "file" or self.nodes[operation["parent"]]["kind"] != "directory":
                raise ValueError("model rename requires a file and a directory")
            self._vacant(operation["parent"], operation["name"])
            node.update(parent=operation["parent"], name=operation["name"])
        elif kind in ("unlink", "rmdir"):
            node = self.nodes[label]
            expected = "file" if kind == "unlink" else "directory"
            if label == "root" or node["kind"] != expected:
                raise ValueError("invalid model removal")
            if any(n["parent"] == label for n in self.nodes.values()):
                raise ValueError("model directory is not empty")
            del self.nodes[label]
        else:
            raise ValueError("unsupported model operation")

    def _vacant(self, parent, name):
        if any(n["parent"] == parent and n["name"] == name for n in self.nodes.values()):
            raise ValueError("model name collision")

    def expected(self):
        entries = []
        for label, node in self.nodes.items():
            if label == "root":
                continue
            path = []
            cursor = label
            while cursor != "root":
                current = self.nodes[cursor]
                path.append(current["name"])
                cursor = current["parent"]
            entry = {"path": list(reversed(path)), "kind": node["kind"]}
            if node["kind"] == "file":
                entry["data"] = node["data"].hex()
            entries.append(entry)
        return sorted(entries, key=lambda entry: entry["path"])


def generate(seed, steps):
    runner.scenario.integer(seed, 0, MASK)
    runner.scenario.integer(steps, 64, 256)
    random = Random(seed)
    model = Model()
    operations = []
    serial = 0

    def emit(op):
        model.apply(op)
        operations.append(op)

    def fresh():
        nonlocal serial
        serial += 1
        # Names force multi-leaf directory shapes without many live files.
        return f"o{serial}", f"{serial:04d}-café-" + "n" * 170

    # An explicit ladder guarantees coverage even when a random seed would
    # otherwise omit a family. The generated suffix varies later interactions.
    emit({"op": "mkdir", "label": "d", "parent": "root", "name": "src"})
    emit({"op": "mkdir", "label": "nested", "parent": "d", "name": "nested"})
    emit({"op": "create", "label": "initial", "parent": "nested", "name": "initial", "data": "00ff"})
    emit({"op": "write", "label": "initial", "offset": 4095, "data": "a1b2c3"})
    emit({"op": "truncate", "label": "initial", "size": 4096})
    emit({"op": "rename", "label": "initial", "parent": "root", "name": "moved"})
    emit({"op": "rmdir", "label": "nested"})
    emit({"op": "unlink", "label": "initial"})
    emit({"op": "sync"})
    emit({"op": "remount"})
    for _ in range(24):
        label, name = fresh()
        emit({"op": "create", "label": label, "parent": "root", "name": name,
              "data": bytes([random.next() & 255]).hex()})
    while len(operations) < steps:
        files = sorted(k for k, n in model.nodes.items() if n["kind"] == "file")
        directories = sorted(k for k, n in model.nodes.items() if n["kind"] == "directory")
        empty = [k for k in directories if k != "root" and not any(n["parent"] == k for n in model.nodes.values())]
        choices = ["sync", "remount"]
        if len(files) < 40: choices.append("create")
        if files: choices.extend(("write", "write", "truncate", "rename", "unlink"))
        if len(directories) < 8: choices.append("mkdir")
        if empty: choices.append("rmdir")
        kind = random.pick(choices)
        op = {"op": kind}
        if kind in ("create", "mkdir"):
            label, name = fresh()
            # Random directories are root children, keeping depth bounded.
            op.update(label=label, parent=random.pick(directories) if kind == "create" else "root", name=name)
            if kind == "create":
                op["data"] = bytes(random.next() & 255 for _ in range(random.pick((0, 1, 7, 33)))).hex()
        elif kind in ("write", "truncate", "rename", "unlink"):
            op["label"] = random.pick(files)
            if kind == "write":
                op.update(offset=random.pick((0, 1, 4095, 4096, 8191)),
                    data=bytes(random.next() & 255 for _ in range(random.pick((1, 7, 33)))).hex())
            elif kind == "truncate":
                op["size"] = random.pick((0, 1, 64, 4095, 4096, 4097, 8193))
            elif kind == "rename":
                _, name = fresh()
                op.update(parent=random.pick(directories), name=name)
        elif kind == "rmdir":
            op["label"] = random.pick(empty)
        emit(op)
    return operations


def scenario(seed, steps, prefix, pages):
    runner.scenario.integer(prefix, 1, steps)
    operations = generate(seed, steps)[:prefix]
    model = Model()
    for op in operations:
        model.apply(op)
    value = {"version": 3, "flight_capacity": 32,
        "volume": {"block_size": 4096, "blocks": 512, "region_size": 64,
                   "log_slots": 8, "tree_cache_pages": pages},
        "operations": operations + [{"op": "remount"}], "expected": model.expected()}
    runner.scenario.validate(runner.encoded(value))
    return value


# Extended operation families (generator version 2). Version 1 above keeps its bytes.

def resize(data, size):
    del data[size:]
    data.extend(b"\0" * max(0, size - len(data)))


def overwrite(data, offset, incoming):
    if incoming:
        end = offset + len(incoming)
        data.extend(b"\0" * max(0, end - len(data)))
        data[offset:end] = incoming


def payload(random, count):
    return bytes(random.next() & 255 for _ in range(count)).hex()


def node_path(nodes, label):
    path = []
    while label != "root":
        path.append(nodes[label]["name"])
        label = nodes[label]["parent"]
    return list(reversed(path))


class WindowModel:
    """Committed namespace plus staged deferred work, independent of the intent-log codec.

    fsync acknowledges every staged operation so far; commit publishes all staged
    work; remount without commit keeps exactly the acknowledged prefix.
    """
    def __init__(self):
        self.committed = Model()
        self.staged = []
        self.records = 0
        self.touched = set()

    def unacknowledged(self):
        return sum(not acknowledged for _, acknowledged in self.staged)

    def visible(self, label):
        data = bytearray(self.committed.nodes[label]["data"])
        for op, _ in self.staged:
            if op["label"] == label:
                self._stage(data, op)
        return data

    @staticmethod
    def _stage(data, op):
        if op["op"] == "window_truncate":
            resize(data, op["size"])
        else:
            overwrite(data, op["offset"], bytes.fromhex(op["data"]))

    def apply(self, op):
        kind = op["op"]
        if kind in ("window_write", "window_truncate"):
            node = self.committed.nodes.get(op["label"])
            if node is None or node["kind"] != "file":
                raise ValueError("window work requires a committed file")
            if kind == "window_write" and not op["data"]:
                raise ValueError("empty window write stages nothing")
            if kind == "window_truncate" and op["size"] == len(self.visible(op["label"])):
                raise ValueError("same-size window truncate stages nothing")
            self.staged.append([op, False])
        elif kind == "window_fsync":
            if self.unacknowledged():
                if self.unacknowledged() > 64 or self.records >= 8:
                    raise ValueError("window fsync group exceeds the intent log")
                for item in self.staged:
                    item[1] = True
                self.records += 1
        elif kind == "window_commit":
            self._publish(len(self.staged))
        elif kind == "remount":
            self._publish(len(self.staged) - self.unacknowledged())
        elif self.staged:
            raise ValueError("direct operation while a window is open")
        else:
            self.committed.apply(op)

    def _publish(self, count):
        for op, _ in self.staged[:count]:
            self.touched.add(op["label"])
            self._stage(self.committed.nodes[op["label"]]["data"], op)
        self.staged, self.records = [], 0


class ObjectModel:
    """Independent object graph for linked namespaces and captured views.

    Names are links to objects. Each mutating operation publishes one checkpoint
    generation and stamps operation index + 1 seconds. Captured allocation is
    modeled only for files whose data stays in logical block zero.
    """
    def __init__(self):
        self.generation = 1
        self.next_object = 16
        self.objects = {1: {"kind": "directory", "links": 1, "protection": 0, "created": [0, 0],
                            "modified": [0, 0], "changed": [0, 0], "generation": 1}}
        self.names = {"root": {"object": 1, "parent": None, "name": ""}}
        self.used = {"root"}
        self.snapshot_ids = {}
        self.views = {}
        self.handles = set()
        self.clones = set()
        self.moved = set()

    def path(self, label):
        path = []
        while label != "root":
            path.append(self.names[label]["name"])
            label = self.names[label]["parent"]
        return list(reversed(path))

    def kind(self, label):
        return self.objects[self.names[label]["object"]]["kind"]

    def labels(self, kind):
        return sorted(label for label in self.names if label != "root" and self.kind(label) == kind)

    def children(self, label):
        return [child for child, name in self.names.items() if name["parent"] == label]

    def reaches(self, ancestor, label):
        while label is not None:
            if label == ancestor:
                return True
            label = self.names[label]["parent"]
        return False

    def file_data(self, label):
        return self._live(label, "file")["data"]

    def _live(self, label, kind=None):
        if not isinstance(label, str) or label == "root" or label not in self.names:
            raise ValueError("unknown model label")
        if kind is not None and self.kind(label) != kind:
            raise ValueError("model operation requires a " + kind)
        return self.objects[self.names[label]["object"]]

    def _directory(self, label):
        if label not in self.names or self.kind(label) != "directory":
            raise ValueError("model parent is not a directory")

    def _vacant(self, parent, name):
        if any(entry["parent"] == parent and entry["name"] == name for entry in self.names.values()):
            raise ValueError("model name collision")

    def _insert(self, op):
        if op["label"] in self.used:
            raise ValueError("model label reused")
        self._directory(op["parent"])
        self._vacant(op["parent"], op["name"])

    def _touch(self, label, now):
        self.objects[self.names[label]["object"]].update(modified=now, changed=now, generation=self.generation)

    def _bind(self, op, identity, now):
        self.used.add(op["label"])
        self.names[op["label"]] = {"object": identity, "parent": op["parent"], "name": op["name"]}
        self._touch(op["parent"], now)

    def _new(self, kind, now, **fields):
        self.generation += 1
        identity = self.next_object
        self.next_object += 1
        self.objects[identity] = dict({"kind": kind, "links": 1, "protection": 0, "created": now,
                                       "modified": now, "changed": now, "generation": self.generation}, **fields)
        return identity

    def apply(self, op, index):
        now = [index + 1, 0]
        kind = op["op"]
        if kind == "sync":
            return
        if kind == "remount":
            self.handles.clear()
        elif kind.startswith("snapshot_"):
            self._snapshot(kind, op["label"])
        elif kind in ("create", "mkdir", "symlink"):
            self._insert(op)
            extra = {}
            if kind == "create":
                data = bytearray.fromhex(op["data"])
                extra = {"data": data, "blocks": set(range(-(-len(data) // BLOCK)))}
            elif kind == "symlink":
                extra = {"target": op["target"]}
            identity = self._new({"create": "file", "mkdir": "directory", "symlink": "symlink"}[kind], now, **extra)
            self._bind(op, identity, now)
        elif kind == "link":
            self._insert(op)
            source = self._live(op["source"], "file")
            self.generation += 1
            source.update(links=source["links"] + 1, changed=now)
            self._bind(op, self.names[op["source"]]["object"], now)
        elif kind == "clone_file":
            self._insert(op)
            source = self._live(op["source"], "file")
            # The executable CloneFile contract copies bytes, size, protection and
            # modification time into a new object with its own link and birth time.
            identity = self._new("file", now, protection=source["protection"], modified=source["modified"],
                                 data=bytearray(source["data"]),
                                 blocks=None if source["blocks"] is None else set(source["blocks"]))
            source["changed"] = now
            self.clones.add(identity)
            self._bind(op, identity, now)
        elif kind == "clone_range":
            source, destination = self._live(op["source"], "file"), self._live(op["destination"], "file")
            if source is destination:
                raise ValueError("model clone range requires distinct objects")
            start, target, length = op["source_offset"], op["destination_offset"], op["length"]
            if start % BLOCK != target % BLOCK or start + length > len(source["data"]):
                raise ValueError("model clone range alignment or source bound")
            if length:
                self.generation += 1
                overwrite(destination["data"], target, bytes(source["data"][start:start + length]))
                destination.update(modified=now, changed=now, generation=self.generation, blocks=None)
                end = target + length
                if min(-(-target // BLOCK) * BLOCK, end) < end - end % BLOCK:
                    # Sharing complete blocks may rewrite the source layout; that
                    # change time depends on the source layout and is not modeled.
                    source["changed"] = None
                self.clones.add(self.names[op["destination"]]["object"])
        elif kind in ("write", "truncate"):
            node = self._live(op["label"], "file")
            data = node["data"]
            if kind == "write":
                incoming = bytes.fromhex(op["data"])
                if not incoming:
                    return
                overwrite(data, op["offset"], incoming)
                touched = set(range(op["offset"] // BLOCK, -(-(op["offset"] + len(incoming)) // BLOCK)))
            else:
                if op["size"] == len(data):
                    return
                resize(data, op["size"])
                touched = set()
            self.generation += 1
            node.update(modified=now, changed=now, generation=self.generation)
            if node["blocks"] is not None:
                retained = -(-len(data) // BLOCK)
                node["blocks"] = {block for block in node["blocks"] | touched if block < retained}
        elif kind == "rename":
            node = self._live(op["label"])
            old = self.names[op["label"]]
            self._directory(op["parent"])
            if (old["parent"], old["name"]) == (op["parent"], op["name"]):
                raise ValueError("model rename changes nothing")
            self._vacant(op["parent"], op["name"])
            if node["kind"] == "directory" and self.reaches(op["label"], op["parent"]):
                raise ValueError("model directory cannot move below itself")
            self.generation += 1
            node["changed"] = now
            self._touch(old["parent"], now)
            self._touch(op["parent"], now)
            old.update(parent=op["parent"], name=op["name"])
            if node["kind"] == "directory":
                self.moved.add(op["label"])
        elif kind in ("unlink", "unlink_symlink", "rmdir"):
            required = {"unlink": "file", "unlink_symlink": "symlink", "rmdir": "directory"}[kind]
            node = self._live(op["label"], required)
            if kind == "rmdir" and self.children(op["label"]):
                raise ValueError("model directory is not empty")
            self.generation += 1
            self._touch(self.names[op["label"]]["parent"], now)
            node["links"] -= 1
            if node["links"]:
                node["changed"] = now
            else:
                del self.objects[self.names[op["label"]]["object"]]
            del self.names[op["label"]]
        elif kind == "set_protection":
            node = self._live(op["label"])
            if node["protection"] != op["protection"]:
                self.generation += 1
                node.update(protection=op["protection"], changed=now)
        else:
            raise ValueError("unsupported model operation")

    def _snapshot(self, kind, label):
        registered = self.views.get(label) is not None
        if kind == "snapshot_create":
            if label in self.snapshot_ids:
                raise ValueError("model snapshot label reused")
            if sum(view is not None for view in self.views.values()) >= SNAPSHOT_LIMITS["max_views"]:
                raise ValueError("model snapshot view limit")
            # Identities are never reused, including after deletion.
            self.snapshot_ids[label] = len(self.snapshot_ids) + 1
            self.views[label] = self.capture(self.snapshot_ids[label])
            self.generation += 1
        elif not registered:
            raise ValueError("model snapshot is not registered")
        elif kind == "snapshot_open":
            if label in self.handles:
                raise ValueError("model snapshot handle is open")
            self.handles.add(label)
        elif kind in ("snapshot_inspect", "snapshot_close"):
            if label not in self.handles:
                raise ValueError("model snapshot handle is not open")
            if kind == "snapshot_close":
                self.handles.remove(label)
        elif kind == "snapshot_delete":
            if label in self.handles:
                raise ValueError("model snapshot handle is open")
            self.generation += 1
            self.views[label] = None
        else:
            raise ValueError("unsupported model snapshot operation")

    def metadata(self, identity):
        node = self.objects[identity]
        if node["changed"] is None:
            raise ValueError("captured metadata outside the model")
        kind = node["kind"]
        size = len(node["data"]) if kind == "file" else len(node["target"].encode()) if kind == "symlink" else 0
        if identity == 1:
            allocated = BLOCK
        elif kind == "file":
            if node["blocks"] is None or not node["blocks"] <= {0}:
                raise ValueError("captured allocation outside the single-block model")
            allocated = BLOCK * len(node["blocks"])
        else:
            allocated = 0
        return {"object_id": identity, "kind": kind, "size": size, "allocated": allocated,
                "links": node["links"], "protection": node["protection"], "created": list(node["created"]),
                "modified": list(node["modified"]), "changed": list(node["changed"]),
                "content_generation": node["generation"]}

    def capture(self, identity):
        entries = []
        for label, name in self.names.items():
            if label == "root":
                continue
            node = self.objects[name["object"]]
            metadata = self.metadata(name["object"])
            data = (node["data"].hex() if node["kind"] == "file"
                    else node["target"].encode().hex() if node["kind"] == "symlink" else "")
            allocation = ([{"offset": 0, "length": BLOCK, "unwritten": False}]
                          if node["kind"] == "file" and metadata["allocated"] else [])
            entries.append({"path": self.path(label), "metadata": metadata, "data": data, "allocation": allocation})
        return {"id": identity, "generation": self.generation, "committed_tx_id": self.generation,
                "root": self.metadata(1), "entries": sorted(entries, key=lambda entry: entry["path"])}

    def entries(self):
        """File/directory projection of scenario versions 1-7."""
        result = []
        for label, name in self.names.items():
            if label == "root":
                continue
            node = self.objects[name["object"]]
            if node["kind"] == "symlink":
                raise ValueError("file/directory projection cannot express a symlink")
            entry = {"path": self.path(label), "kind": node["kind"]}
            if node["kind"] == "file":
                entry["data"] = node["data"].hex()
            result.append(entry)
        return sorted(result, key=lambda entry: entry["path"])

    def linked(self):
        """Version-9 projection: a file alias is the first sorted path naming its object."""
        first, result = {}, []
        labels = sorted((label for label in self.names if label != "root"), key=self.path)
        for index, label in enumerate(labels):
            identity = self.names[label]["object"]
            node = self.objects[identity]
            entry = {"path": self.path(label), "kind": node["kind"], "links": node["links"],
                     "protection": node["protection"]}
            if node["kind"] == "file":
                entry.update(alias=first.setdefault(identity, index), data=node["data"].hex())
            elif node["kind"] == "symlink":
                entry["target"] = node["target"]
            result.append(entry)
        return result

    def snapshots(self):
        return sorted((view for view in self.views.values() if view is not None), key=lambda view: view["id"])


def directory_moves(model, directories, max_depth):
    """Cross-directory moves that never enter the moved subtree or exceed the depth bound."""
    def height(label):
        return max((1 + height(child) for child in model.children(label) if model.kind(child) == "directory"),
                   default=0)
    return [(label, parent) for label in directories[1:] for parent in directories
            if parent != model.names[label]["parent"] and not model.reaches(label, parent)
            and len(model.path(parent)) + 1 + height(label) <= max_depth]


def generate_window(seed, steps):
    random = Random(seed)
    model = WindowModel()
    operations = []
    serial = 0

    def emit(op):
        model.apply(op)
        operations.append(op)

    def fresh():
        nonlocal serial
        serial += 1
        return f"w{serial}", f"{serial:04d}-fenêtre-" + "w" * 120

    # Ladder: acknowledged prefix across remount, commit of acknowledged and
    # unacknowledged work, and loss of unacknowledged work at remount.
    emit({"op": "mkdir", "label": "d", "parent": "root", "name": "window"})
    emit({"op": "create", "label": "a", "parent": "root", "name": "a", "data": "00ff"})
    emit({"op": "create", "label": "b", "parent": "d", "name": "b", "data": "11" * 33})
    emit({"op": "sync"})
    emit({"op": "window_write", "label": "a", "offset": 4095, "data": "a1b2c3"})
    emit({"op": "window_truncate", "label": "b", "size": 1})
    emit({"op": "window_fsync"})
    emit({"op": "window_write", "label": "a", "offset": 0, "data": "5a"})
    emit({"op": "remount"})
    emit({"op": "window_write", "label": "b", "offset": 8191, "data": "c4"})
    emit({"op": "window_truncate", "label": "a", "size": 64})
    emit({"op": "window_fsync"})
    emit({"op": "window_write", "label": "b", "offset": 0, "data": "d5"})
    emit({"op": "window_fsync"})
    emit({"op": "window_commit"})
    emit({"op": "window_write", "label": "a", "offset": 1, "data": "e6"})
    emit({"op": "window_commit"})
    emit({"op": "window_truncate", "label": "b", "size": 4097})
    emit({"op": "remount"})
    for _ in range(8):
        label, name = fresh()
        emit({"op": "create", "label": label, "parent": "root", "name": name, "data": payload(random, 1)})
    while len(operations) < steps:
        nodes = model.committed.nodes
        files = sorted(k for k, n in nodes.items() if n["kind"] == "file")
        directories = sorted(k for k, n in nodes.items() if n["kind"] == "directory")
        if model.staged:
            choices = ["window_fsync", "window_fsync", "window_commit", "remount"]
            if model.unacknowledged() < 8 and len(model.staged) < 24:
                choices += ["window_write"] * 3 + ["window_truncate"] * 2
            if model.records >= 6 and model.unacknowledged():
                choices = [choice for choice in choices if choice != "window_fsync"]
        else:
            empty = [k for k in directories if k != "root" and not any(n["parent"] == k for n in nodes.values())]
            # Commit and fsync without a window are admitted no-ops.
            choices = ["sync", "remount", "window_commit", "window_fsync"]
            if len(files) < 16: choices.append("create")
            if files: choices += ["write", "truncate", "rename", "unlink"] + ["window_write"] * 3 + ["window_truncate"] * 2
            if len(directories) < 4: choices.append("mkdir")
            if empty: choices.append("rmdir")
        kind = random.pick(choices)
        op = {"op": kind}
        if kind in ("create", "mkdir"):
            label, name = fresh()
            op.update(label=label, parent=random.pick(directories) if kind == "create" else "root", name=name)
            if kind == "create":
                op["data"] = payload(random, random.pick((0, 1, 33)))
        elif kind in ("write", "window_write"):
            offset = random.pick((0, 1, 4095, 4096, 8191))
            count = random.pick(tuple(c for c in (1, 7, 33, 300) if offset + c <= MAX_FILE_BYTES))
            op.update(label=random.pick(files), offset=offset, data=payload(random, count))
        elif kind in ("truncate", "window_truncate"):
            label = random.pick(files)
            current = len(model.visible(label))
            op.update(label=label, size=random.pick(tuple(s for s in (0, 1, 64, 4095, 4096, 4097, 8193) if s != current)))
        elif kind == "rename":
            _, name = fresh()
            op.update(label=random.pick(files), parent=random.pick(directories), name=name)
        elif kind == "unlink":
            op["label"] = random.pick(files)
        elif kind == "rmdir":
            op["label"] = random.pick(empty)
        emit(op)
    return operations


def generate_snapshot(seed, steps):
    random = Random(seed)
    model = ObjectModel()
    operations = []
    serial = 0

    def emit(op):
        model.apply(op, len(operations))
        operations.append(op)

    def fresh(prefix):
        nonlocal serial
        serial += 1
        return f"{prefix}{serial}"

    # Ladder: handles across mutation, inspection, remount and deletion; file and
    # directory moves, sparse growth and captured unlinked content.
    emit({"op": "mkdir", "label": "d", "parent": "root", "name": "d"})
    emit({"op": "create", "label": "f", "parent": "d", "name": "f", "data": "61"})
    emit({"op": "snapshot_create", "label": "s1"})
    emit({"op": "snapshot_open", "label": "s1"})
    emit({"op": "write", "label": "f", "offset": 0, "data": "62"})
    emit({"op": "rename", "label": "f", "parent": "root", "name": "g"})
    emit({"op": "snapshot_inspect", "label": "s1"})
    emit({"op": "snapshot_close", "label": "s1"})
    emit({"op": "mkdir", "label": "e", "parent": "root", "name": "e"})
    emit({"op": "rename", "label": "d", "parent": "e", "name": "d2"})
    emit({"op": "create", "label": "h", "parent": "root", "name": "h", "data": ""})
    emit({"op": "truncate", "label": "h", "size": 100})
    emit({"op": "snapshot_create", "label": "s2"})
    emit({"op": "write", "label": "h", "offset": 50, "data": "63"})
    emit({"op": "unlink", "label": "h"})
    emit({"op": "remount"})
    emit({"op": "snapshot_open", "label": "s2"})
    emit({"op": "snapshot_inspect", "label": "s2"})
    emit({"op": "snapshot_close", "label": "s2"})
    emit({"op": "snapshot_delete", "label": "s1"})
    emit({"op": "truncate", "label": "f", "size": 4096})
    emit({"op": "rmdir", "label": "d"})
    emit({"op": "sync"})
    emit({"op": "remount"})
    for _ in range(6):
        label = fresh("n")
        emit({"op": "create", "label": label, "parent": "root", "name": label,
              "data": payload(random, random.pick((0, 1, 33)))})
    while len(operations) < steps:
        files = model.labels("file")
        directories = ["root"] + model.labels("directory")
        shallow = [d for d in directories if len(model.path(d)) < 2]
        empty = [d for d in directories[1:] if not model.children(d)]
        moves = directory_moves(model, directories, 2)
        live = sorted(label for label, view in model.views.items() if view is not None)
        closed = [label for label in live if label not in model.handles]
        opened = sorted(model.handles)
        choices = ["sync", "remount"]
        if len(files) < 12: choices += ["create", "create"]
        if files: choices += ["write", "write", "truncate", "rename", "unlink"]
        if len(directories) < 6: choices.append("mkdir")
        if empty: choices.append("rmdir")
        if moves: choices.append("move")
        if len(live) < 6: choices += ["snapshot_create", "snapshot_create"]
        if closed: choices += ["snapshot_open", "snapshot_delete"]
        if opened: choices += ["snapshot_inspect", "snapshot_close"]
        kind = random.pick(choices)
        if kind in ("create", "mkdir"):
            label = fresh("n")
            op = {"op": kind, "label": label, "parent": random.pick(directories if kind == "create" else shallow),
                  "name": label}
            if kind == "create":
                op["data"] = payload(random, random.pick((0, 1, 33)))
        elif kind == "write":
            offset = random.pick((0, 1, 100, 4000, 4095))
            op = {"op": kind, "label": random.pick(files), "offset": offset,
                  "data": payload(random, random.pick(tuple(c for c in (1, 7, 33) if offset + c <= BLOCK)))}
        elif kind == "truncate":
            label = random.pick(files)
            current = len(model.file_data(label))
            op = {"op": kind, "label": label, "size": random.pick(tuple(s for s in (0, 1, 64, 4000, 4096) if s != current))}
        elif kind in ("rename", "move"):
            label, parent = (random.pick(files), random.pick(directories)) if kind == "rename" else random.pick(moves)
            op = {"op": "rename", "label": label, "parent": parent, "name": fresh("n")}
        elif kind in ("unlink", "rmdir"):
            op = {"op": kind, "label": random.pick(files if kind == "unlink" else empty)}
        elif kind == "snapshot_create":
            op = {"op": kind, "label": fresh("s")}
        elif kind in ("snapshot_open", "snapshot_delete"):
            op = {"op": kind, "label": random.pick(closed)}
        elif kind in ("snapshot_inspect", "snapshot_close"):
            op = {"op": kind, "label": random.pick(opened)}
        else:
            op = {"op": kind}
        emit(op)
    return operations


def generate_namespace(seed, steps):
    random = Random(seed)
    model = ObjectModel()
    operations = []
    serial = 0

    def emit(op):
        model.apply(op, len(operations))
        operations.append(op)

    def fresh():
        nonlocal serial
        serial += 1
        return f"k{serial}", f"{serial:04d}-lien-" + "l" * 48

    # Ladder: aliases written through another name, symlink and directory moves,
    # inherited protection, unaligned CloneRange and independent clone bytes.
    emit({"op": "mkdir", "label": "a", "parent": "root", "name": "alpha"})
    emit({"op": "mkdir", "label": "b", "parent": "a", "name": "beta"})
    emit({"op": "create", "label": "f", "parent": "a", "name": "file", "data": payload(random, 5000)})
    emit({"op": "link", "label": "l", "source": "f", "parent": "b", "name": "hard"})
    emit({"op": "write", "label": "l", "offset": 4095, "data": "a1b2c3"})
    emit({"op": "symlink", "label": "s", "parent": "root", "name": "sym", "target": "alpha/file"})
    emit({"op": "set_protection", "label": "f", "protection": 5})
    emit({"op": "clone_file", "label": "c", "source": "f", "parent": "root", "name": "clone"})
    emit({"op": "clone_range", "source": "f", "source_offset": 1, "destination": "c",
          "destination_offset": 4097, "length": 4000})
    emit({"op": "write", "label": "f", "offset": 0, "data": "ff"})
    emit({"op": "rename", "label": "b", "parent": "root", "name": "beta2"})
    emit({"op": "rename", "label": "s", "parent": "b", "name": "sym2"})
    emit({"op": "set_protection", "label": "b", "protection": 0o755})
    emit({"op": "unlink", "label": "f"})
    emit({"op": "unlink_symlink", "label": "s"})
    emit({"op": "truncate", "label": "c", "size": 1})
    emit({"op": "link", "label": "m", "source": "c", "parent": "a", "name": "second"})
    emit({"op": "symlink", "label": "t", "parent": "b", "name": "target", "target": "../ünïcode/ταξί"})
    emit({"op": "sync"})
    emit({"op": "remount"})
    for _ in range(4):
        label, name = fresh()
        emit({"op": "create", "label": label, "parent": "root", "name": name,
              "data": payload(random, random.pick((0, 1, 33)))})
    while len(operations) < steps:
        files = model.labels("file")
        symlinks = model.labels("symlink")
        directories = ["root"] + model.labels("directory")
        shallow = [d for d in directories if len(model.path(d)) < 3]
        empty = [d for d in directories[1:] if not model.children(d)]
        moves = directory_moves(model, directories, 3)
        pairs = [(source, destination) for source in files for destination in files
                 if model.names[source]["object"] != model.names[destination]["object"] and model.file_data(source)]
        choices = ["sync", "remount"]
        if len(model.names) <= 48:
            choices += ["create", "create", "symlink"]
            if len(directories) < 8: choices.append("mkdir")
            if files: choices += ["link", "clone_file"]
        if files: choices += ["write", "write", "truncate", "unlink", "rename", "set_protection"]
        if pairs: choices += ["clone_range", "clone_range"]
        if symlinks: choices += ["unlink_symlink", "rename_symlink"]
        if empty: choices.append("rmdir")
        if moves: choices.append("move")
        if len(directories) > 1: choices.append("protect_directory")
        kind = random.pick(choices)
        if kind in ("create", "mkdir", "symlink"):
            label, name = fresh()
            op = {"op": kind, "label": label, "parent": random.pick(shallow if kind == "mkdir" else directories),
                  "name": name}
            if kind == "create":
                op["data"] = payload(random, random.pick((0, 1, 7, 33, 5000)))
            elif kind == "symlink":
                op["target"] = random.pick(SYMLINK_TARGETS)
        elif kind in ("link", "clone_file"):
            label, name = fresh()
            op = {"op": kind, "label": label, "source": random.pick(files), "parent": random.pick(directories),
                  "name": name}
        elif kind == "write":
            offset = random.pick((0, 1, 4095, 4096, 8191))
            op = {"op": kind, "label": random.pick(files), "offset": offset,
                  "data": payload(random, random.pick(tuple(c for c in (1, 7, 33, 300) if offset + c <= MAX_FILE_BYTES)))}
        elif kind == "truncate":
            label = random.pick(files)
            current = len(model.file_data(label))
            op = {"op": kind, "label": label,
                  "size": random.pick(tuple(s for s in (0, 1, 64, 4095, 4096, 4097, 8193) if s != current))}
        elif kind in ("unlink", "unlink_symlink"):
            op = {"op": kind, "label": random.pick(files if kind == "unlink" else symlinks)}
        elif kind in ("rename", "rename_symlink", "move"):
            if kind == "move":
                label, parent = random.pick(moves)
            else:
                label, parent = random.pick(files if kind == "rename" else symlinks), random.pick(directories)
            op = {"op": "rename", "label": label, "parent": parent, "name": fresh()[1]}
        elif kind in ("set_protection", "protect_directory"):
            op = {"op": "set_protection", "label": random.pick(files if kind == "set_protection" else directories[1:]),
                  "protection": random.pick(PROTECTIONS)}
        elif kind == "clone_range":
            source, destination = random.pick(pairs)
            size = len(model.file_data(source))
            start = random.pick(tuple(o for o in (0, 1, 4095, 4096, 4097) if o < size))
            target = random.pick((start % BLOCK, start % BLOCK + BLOCK))
            lengths = tuple(n for n in (1, 7, 100, 4096, 5000) if start + n <= size and target + n <= MAX_FILE_BYTES)
            op = {"op": kind, "source": source, "source_offset": start, "destination": destination,
                  "destination_offset": target,
                  "length": random.pick(lengths) if lengths else min(size - start, MAX_FILE_BYTES - target)}
        elif kind == "rmdir":
            op = {"op": kind, "label": random.pick(empty)}
        else:
            op = {"op": kind}
        emit(op)
    return operations


GENERATORS = {"window": generate_window, "snapshot": generate_snapshot, "namespace": generate_namespace}


def generate_family(family, seed, steps):
    if family not in FAMILY_VERSIONS:
        raise ValueError("unknown operation family")
    runner.scenario.integer(seed, 0, MASK)
    runner.scenario.integer(steps, 64, 256)
    return GENERATORS[family](seed, steps)


def family_case(family, seed, steps, prefix, pages):
    """Scenario plus the independent model that computed its exact expected state."""
    runner.scenario.integer(prefix, 1, steps)
    operations = generate_family(family, seed, steps)[:prefix] + [{"op": "remount"}]
    value = {"version": FAMILY_VERSIONS[family],
             "volume": {"block_size": 4096, "blocks": 512, "region_size": 64, "log_slots": 8,
                        "tree_cache_pages": pages},
             "flight_capacity": 32, "flight_categories": 63 if family == "window" else 127,
             "flight_sink": None, "operations": operations}
    if family == "window":
        model = WindowModel()
        for op in operations:
            model.apply(op)
        value["expected"] = model.committed.expected()
    else:
        model = ObjectModel()
        for index, op in enumerate(operations):
            model.apply(op, index)
        value.update(expected=model.entries() if family == "snapshot" else model.linked(),
                     snapshot_limits=dict(SNAPSHOT_LIMITS), expected_snapshots=model.snapshots())
    runner.scenario.validate(runner.encoded(value))
    return value, model


def family_scenario(family, seed, steps, prefix, pages):
    return family_case(family, seed, steps, prefix, pages)[0]


def flip(data):
    return f"{int(data[:2], 16) ^ 0xFF:02x}" + data[2:]


def apply_control(control, value, model):
    """Make exactly one expected value of the family wrong; return its location or None."""
    def first(entries, predicate):
        return next((entry for entry in entries if predicate(entry)), None)
    expected = value["expected"]
    if control == "snapshot-entry":
        for view in value["expected_snapshots"]:
            entry = first(view["entries"], lambda e: e["metadata"]["kind"] == "file" and e["data"])
            if entry is not None:
                entry["data"] = flip(entry["data"])
                return [str(view["id"])] + entry["path"]
        return None
    if control == "window-byte":
        paths = [node_path(model.committed.nodes, label) for label in sorted(model.touched)
                 if label in model.committed.nodes]
        entry = first(expected, lambda e: e["kind"] == "file" and e["data"] and e["path"] in paths)
    elif control == "link-count":
        entry = first(expected, lambda e: e["kind"] == "file" and e["links"] > 1)
    elif control == "symlink-target":
        entry = first(expected, lambda e: e["kind"] == "symlink")
    elif control == "protection":
        entry = first(expected, lambda e: e["protection"])
    elif control == "clone-byte":
        paths = [model.path(label) for label in sorted(model.names)
                 if label != "root" and model.names[label]["object"] in model.clones]
        entry = first(expected, lambda e: e["kind"] == "file" and e["data"] and e["path"] in paths)
    elif control == "directory-rename":
        paths = [model.path(label) for label in sorted(model.moved) if label in model.names]
        entry = first(expected, lambda e: e["kind"] == "directory" and e["path"] in paths)
    else:
        raise ValueError("unknown negative control")
    if entry is None:
        return None
    location = entry["path"]
    if control in ("window-byte", "clone-byte"):
        entry["data"] = flip(entry["data"])
    elif control == "link-count":
        entry["links"] -= 1
    elif control == "symlink-target":
        entry["target"] += "~"
    elif control == "protection":
        entry["protection"] ^= 1
    else:
        entry["path"] = location[:-1] + [location[-1] + "~"]
    return location


def expected_state(value):
    return value["expected"], value.get("expected_snapshots")


def plan_cases(seeds, steps, family=None, control=None):
    """Compute every exact expected state before execution; cache policy never changes it."""
    cases, applied = [], None
    for seed in seeds:
        for prefix in (steps // 2, steps):
            built = [(pages, scenario(seed, steps, prefix, pages), None) if family is None
                     else (pages, *family_case(family, seed, steps, prefix, pages)) for pages in PROFILES]
            if any(expected_state(value) != expected_state(built[0][1]) for _, value, _ in built):
                raise ValueError("expected state differs across cache profiles")
            for pages, value, model in built:
                name = f"seed-{seed}-prefix-{prefix}-cache-{pages}"
                if control is not None and applied is None:
                    location = apply_control(control, value, model)
                    if location is not None:
                        runner.scenario.validate(runner.encoded(value))
                        applied = {"kind": control, "case": name, "location": location}
                cases.append((name, value))
    if control is not None and applied is None:
        raise ValueError("negative control found no applicable expected value")
    return cases, applied


def publish_json(path, value):
    # Exclusive, durable small records; never replace evidence from an earlier run.
    with Path(path).open("xb") as output:
        output.write(runner.encoded(value))
        output.flush()
        os.fsync(output.fileno())
    fd = runner.bundle._directory(Path(path).parent)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def qualify(output, binary, seeds, steps, bundle_payload_bytes=512 * 1024 * 1024, family=None, control=None):
    runner.scenario.integer(steps, 64, 256)
    runner.scenario.integer(bundle_payload_bytes, 1, 4 * 1024**3)
    if not isinstance(seeds, list) or not 1 <= len(seeds) <= 16 or len(set(seeds)) != len(seeds):
        raise ValueError("need 1..16 distinct seeds")
    for seed in seeds: runner.scenario.integer(seed, 0, MASK)
    if family is not None and family not in FAMILY_VERSIONS:
        raise ValueError("unknown operation family")
    if control is not None and (family is None or control not in CONTROLS[family]):
        raise ValueError("negative control does not belong to the selected family")
    cases, applied = plan_cases(seeds, steps, family, control)
    binary = Path(binary).resolve(strict=True)
    source = runner.source_identity()
    binary_hash = runner.executable_digest(binary)
    output = Path(output).absolute()
    destination = output.resolve()
    if destination.is_relative_to(runner.ROOT.resolve()):
        ignored = subprocess.run(["git", "check-ignore", "--quiet", "--", str(destination)],
                                 cwd=runner.ROOT, check=False)
        if ignored.returncode != 0:
            raise ValueError("output would overlap non-ignored source files")
    os.mkdir(output, 0o700)
    parent = runner.bundle._directory(output.parent)
    try: os.fsync(parent)
    finally: os.close(parent)
    recipe = {"version": VERSION, "generator_sha256": runner.executable_digest(Path(__file__)),
              "seeds": seeds, "steps": steps, "prefixes": [steps // 2, steps],
              "profiles": list(PROFILES), "source_observed": source, "runner_sha256": binary_hash,
              "bundle_payload_bytes": bundle_payload_bytes}
    if family is not None:
        recipe.update(version=EXTENDED_VERSION, family=family, scenario_version=FAMILY_VERSIONS[family],
                      negative_control=applied)
    publish_json(output / "recipe.json", recipe)
    recipe_hash = runner.executable_digest(output / "recipe.json")
    results = []
    payload_bytes = 0
    try:
        for name, value in cases:
            publish_json(output / (name + ".json"), value)
            records, success = runner.execute(runner.encoded(value), binary)
            identity = json.loads(records["run.json"])
            if identity["source_observed"] != source or identity["runner_sha256"] != binary_hash:
                raise ValueError("qualification source or executable changed")
            size = sum(map(len, records.values()))
            if payload_bytes + size > bundle_payload_bytes:
                raise ValueError("bundle payload budget exhausted; scenario retained without full runner artifacts")
            runner.bundle.publish(output / name, records)
            if runner.bundle.read_bundle(output / name) != records:
                raise ValueError("published bundle differs from executed artifacts")
            payload_bytes += size
            results.append({"case": name, "success": success,
                "manifest_sha256": runner.executable_digest(output / name / "manifest.json")})
            print(json.dumps(results[-1], sort_keys=True), flush=True)
            if not success:
                publish_json(output / "result.json", {"outcome": "failure", "recipe_sha256": recipe_hash, "bundle_payload_bytes": payload_bytes, "cases": results})
                return False
        publish_json(output / "result.json", {"outcome": "pass", "recipe_sha256": recipe_hash, "bundle_payload_bytes": payload_bytes, "cases": results})
        parent = runner.bundle._directory(output.parent)
        try: os.fsync(parent)
        finally: os.close(parent)
        return True
    except Exception as error:
        publish_json(output / "error.json", {"outcome": "incomplete", "recipe_sha256": recipe_hash, "error": str(error), "cases": results})
        raise


def control_reproduced(output):
    """A negative-control campaign succeeds only by failing at its mutated case."""
    recipe = json.loads((Path(output) / "recipe.json").read_text())
    result = json.loads((Path(output) / "result.json").read_text())
    control = recipe.get("negative_control")
    return (control is not None and result["outcome"] == "failure" and bool(result["cases"])
            and result["cases"][-1]["case"] == control["case"] and not result["cases"][-1]["success"])


def replay_campaign(output, binary):
    """Replay each retained case in a fresh process; it must reproduce its recorded verdict."""
    output = Path(output)
    result = json.loads((output / "result.json").read_text())
    if result.get("outcome") not in ("pass", "failure"):
        raise ValueError("campaign has no completion record")
    for case in result["cases"]:
        path = output / case["case"]
        if runner.executable_digest(path / "manifest.json") != case["manifest_sha256"]:
            raise ValueError("retained manifest differs from the completion record")
        process = subprocess.run([sys.executable, str(Path(__file__).with_name("afsptest.py")), "--runner",
                                  str(binary), "replay", str(path)], capture_output=True, timeout=900, check=False)
        if process.returncode != (0 if case["success"] else 2):
            raise ValueError(f"fresh replay of {case['case']} exited {process.returncode}: "
                             + process.stderr.decode(errors="replace")[-400:])
        print(json.dumps({"case": case["case"], "reproduced": "pass" if case["success"] else "failure"},
                         sort_keys=True), flush=True)
    return len(result["cases"])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--runner", type=Path, default=runner.ROOT / "target/debug/afsplus-scenario")
    parser.add_argument("--seeds", type=int, nargs="+", default=[1, 7, 42])
    parser.add_argument("--steps", type=int, default=96)
    parser.add_argument("--bundle-payload-mib", type=int, default=512)
    parser.add_argument("--family", choices=sorted(FAMILY_VERSIONS))
    parser.add_argument("--negative-control", choices=sorted({c for group in CONTROLS.values() for c in group}))
    parser.add_argument("--replay", action="store_true",
                        help="replay every retained case of an existing campaign in fresh processes")
    args = parser.parse_args()
    try:
        if args.replay:
            print(json.dumps({"replayed_cases": replay_campaign(args.output, args.runner)}), flush=True)
            return 0
        passed = qualify(args.output, args.runner, args.seeds, args.steps, args.bundle_payload_mib * 1024**2,
                         args.family, args.negative_control)
        if args.negative_control is None:
            return 0 if passed else 1
        return 0 if control_reproduced(args.output) else 1
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        parser.exit(1, f"semantic qualification refused: {error}\n")


if __name__ == "__main__":
    raise SystemExit(main())
