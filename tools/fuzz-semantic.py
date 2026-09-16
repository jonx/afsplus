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
FAMILY_VERSIONS = {"window": 5, "snapshot": 7, "namespace": 9, "replace": 9, "orphan": 9,
                   "space": 9, "batch": 9, "maintenance": 9, "captured": 9}
CONTROLS = {"window": ("window-byte",), "snapshot": ("snapshot-entry",),
            "namespace": ("link-count", "symlink-target", "protection", "clone-byte", "directory-rename"),
            "replace": ("replaced-byte",), "orphan": ("orphan-count", "orphan-bytes"),
            "space": ("reservation", "policy-flag"),
            "batch": ("batch-path", "orphan-count", "orphan-bytes"),
            "maintenance": ("maintenance-entry",),
            "captured": ("captured-coverage", "clone-changed")}
# Version-9 volume feature and orphan cleanup budget per family.
FAMILY_POLICY = {"space": True}
FAMILY_ORPHAN_EXTENTS = {"orphan": 2}
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


def coverage(blocks):
    """Normalized logical allocation: maximal intervals of equal reservation.

    Adjacent blocks merge when their unwritten flag agrees, so the result
    depends on which logical bytes are reserved and whether they read as
    zeros, never on extent record boundaries or physical placement.
    """
    merged = []
    for block in sorted(blocks):
        flag = blocks[block]
        if merged and merged[-1]["offset"] + merged[-1]["length"] == block * BLOCK and merged[-1]["unwritten"] == flag:
            merged[-1]["length"] += BLOCK
        else:
            merged.append({"offset": block * BLOCK, "length": BLOCK, "unwritten": flag})
    return merged


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
    def __init__(self, single_block=False, orphan_extents=1):
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
        # Sources whose layout a CloneRange marked, and whose change time
        # therefore moved with that call.
        self.clone_sources = set()
        self.moved = set()
        # Version-9 state: reserved-directory members, per-file policy and the
        # cleanup budget in whole extent records (ADR-066).
        self.single_block = single_block
        self.orphan_extents = orphan_extents
        self.orphaned = {}
        self.replaced = set()
        self.batched = set()
        self.maintained = False
        # Staged window namespace groups and their acknowledging log records.
        self.staged = []
        self.records = 0

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
                                       "modified": now, "changed": now, "generation": self.generation,
                                       "policy": False}, **fields)
        return identity

    def apply(self, op, index):
        now = [index + 1, 0]
        kind = op["op"]
        if self.staged and kind not in ("window_batch", "window_fsync", "window_commit", "remount"):
            raise ValueError("model direct operation while a window is open")
        if kind == "sync":
            return
        if kind == "window_batch":
            if not op["items"]:
                raise ValueError("model window batch is empty")
            self.staged.append([op["items"], False])
        elif kind == "window_fsync":
            unacknowledged = sum(len(items) for items, acknowledged in self.staged if not acknowledged)
            if unacknowledged:
                if unacknowledged > 64 or self.records >= 6:
                    raise ValueError("model window fsync group exceeds the intent log")
                for group in self.staged:
                    group[1] = True
                self.records += 1
        elif kind == "window_commit":
            self._publish(len(self.staged), now)
        elif kind == "remount":
            # A remount without a commit keeps exactly the acknowledged prefix.
            self._publish(sum(1 for _, acknowledged in self.staged if acknowledged), now)
            self.handles.clear()
        elif kind in ("reclaim_step", "snapshot_maintenance_step"):
            # Namespace, bytes and captured views are invariant. The volume
            # generation may advance, so no view is captured after a step.
            self.maintained = True
        elif kind.startswith("snapshot_"):
            self._snapshot(kind, op["label"])
        elif kind in ("create", "mkdir", "symlink"):
            self._insert(op)
            extra = {}
            if kind == "create":
                data = bytearray.fromhex(op["data"])
                extra = {"data": data, "shared": set(),
                         "blocks": {block: False for block in range(-(-len(data) // BLOCK))}}
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
                                 data=bytearray(source["data"]), policy=False,
                                 blocks=dict(source["blocks"]), shared=set(source["blocks"]))
            # CloneFile flags every source run shared and rewrites the source
            # record, so the source change time moves with every call.
            source["shared"] = set(source["blocks"])
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
                end = target + length
                first, last = target // BLOCK, -(-end // BLOCK)
                # Complete destination blocks share the corresponding source
                # blocks, so a source hole stays a hole. The at most two partial
                # boundary blocks are copied into private storage, whatever the
                # destination held there before.
                full_start = min(target + -target % BLOCK, end)
                full_end = end - end % BLOCK
                whole = range(full_start // BLOCK, full_end // BLOCK) if full_start < full_end else range(0)
                origin = (start + (full_start - target)) // BLOCK
                blocks = {block: flag for block, flag in destination["blocks"].items()
                          if block < first or block >= last}
                shared = {block for block in destination["shared"] if block < first or block >= last}
                for block in range(first, last):
                    if block not in whole:
                        blocks[block] = False
                        continue
                    origin_block = origin + block - whole.start
                    if origin_block in source["blocks"]:
                        blocks[block] = source["blocks"][origin_block]
                        shared.add(block)
                # The source layout is rewritten exactly when a mapped block of
                # the shared range gains the flag, and that rewrite carries the
                # source change time without touching its modification time.
                marked = {block for block in range(origin, origin + len(whole))
                          if block in source["blocks"] and block not in source["shared"]}
                if marked:
                    source["shared"] |= marked
                    source["changed"] = now
                    self.clone_sources.add(self.names[op["source"]]["object"])
                overwrite(destination["data"], target, bytes(source["data"][start:start + length]))
                destination.update(modified=now, changed=now, generation=self.generation,
                                   blocks=blocks, shared=shared)
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
                shrink = op["size"] < len(data)
                resize(data, op["size"])
                touched = set()
            self.generation += 1
            node.update(modified=now, changed=now, generation=self.generation)
            mapped = dict(node["blocks"])
            # A written block leaves no reservation behind, whole or partial,
            # and its replacement storage is private. A write keeps mappings
            # past the end of file; a truncation releases every block beyond
            # the retained logical size.
            mapped.update({block: False for block in touched})
            shared = node["shared"] - touched
            # A growing truncation releases nothing.
            retained = -(-len(data) // BLOCK) if kind == "truncate" and shrink else None
            if retained is not None:
                mapped = {block: flag for block, flag in mapped.items() if block < retained}
                shared = {block for block in shared if block < retained}
                # A shrink to a partial block rewrites that written block with a
                # zeroed tail into private storage; a reservation is left alone.
                if len(data) % BLOCK and mapped.get(retained - 1) is False:
                    shared.discard(retained - 1)
            node["blocks"], node["shared"] = mapped, shared
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
        elif kind in ("rename_replace", "rename_replace_orphan"):
            node = self._live(op["label"], "file")
            victim = self._live(op["victim"], "file")
            old, target = self.names[op["label"]], self.names[op["victim"]]
            self._directory(op["parent"])
            if (target["parent"], target["name"]) != (op["parent"], op["name"]):
                raise ValueError("model victim does not name the replaced entry")
            if old["object"] == target["object"]:
                raise ValueError("model replacement requires distinct objects")
            self.generation += 1
            node["changed"] = now
            self._touch(old["parent"], now)
            self._touch(op["parent"], now)
            del self.names[op["victim"]]
            old.update(parent=op["parent"], name=op["name"])
            victim["links"] -= 1
            if victim["links"]:
                victim["changed"] = now
            elif kind == "rename_replace_orphan":
                # The reserved directory holds the final link (ADR-066).
                victim.update(links=1, changed=now)
                self.orphaned[op["victim"]] = target["object"]
            else:
                del self.objects[target["object"]]
            self.replaced.add(op["label"])
        elif kind == "orphan_file":
            node = self._live(op["label"], "file")
            if node["links"] != 1:
                raise ValueError("model orphan requires a final visible link")
            self.generation += 1
            self._touch(self.names[op["label"]]["parent"], now)
            node["changed"] = now
            self.orphaned[op["label"]] = self.names[op["label"]]["object"]
            del self.names[op["label"]]
        elif kind == "cleanup_orphan":
            if op["label"] not in self.orphaned:
                raise ValueError("model orphan label is not registered")
            node = self.objects[self.orphaned[op["label"]]]
            # A step removes whole extent records from the logical end and,
            # once the layout is empty, the entry and record in the same call.
            # The model admits only orphans whose layout fits one budget.
            if len(coverage(node["blocks"])) > self.orphan_extents:
                raise ValueError("model orphan exceeds one budgeted cleanup step")
            self.generation += 1
            del self.objects[self.orphaned[op["label"]]]
            del self.orphaned[op["label"]]
        elif kind in ("preallocate", "preallocate_bounded"):
            node = self._live(op["label"], "file")
            if not op["length"]:
                return
            first = op["offset"] // BLOCK
            last = -(-(op["offset"] + op["length"]) // BLOCK)
            if kind == "preallocate_bounded" and last - first > op["max_blocks"]:
                raise ValueError("model preallocation exceeds its block budget")
            holes = [block for block in range(first, last) if block not in node["blocks"]]
            if not holes:
                return
            reserved = dict(node["blocks"])
            reserved.update({block: True for block in holes})
            self.generation += 1
            # Size, contents and modification time are untouched by a reservation.
            node.update(blocks=reserved, changed=now)
        elif kind == "set_data_policy":
            node = self._live(op["label"], "file")
            if node["policy"] != op["policy"]:
                self.generation += 1
                node.update(policy=op["policy"], changed=now)
        elif kind == "restore_metadata":
            node = self._live(op["label"])
            wanted = {"protection": op["protection"], "created": [op["created"], 0],
                      "modified": [op["modified"], 0], "changed": [op["changed"], 0]}
            if any(node[key] != value for key, value in wanted.items()):
                self.generation += 1
                node.update(**wanted)
        elif kind == "batch":
            self._batch(op["items"], now)
        elif kind == "set_protection":
            node = self._live(op["label"])
            if node["protection"] != op["protection"]:
                self.generation += 1
                node.update(protection=op["protection"], changed=now)
        else:
            raise ValueError("unsupported model operation")

    def _publish(self, count, now):
        # One window owns one pending group of creates. A staged unlink of an
        # object that same window created cancels the create, so the reserved
        # directory receives the final unlink of committed objects alone.
        window_created = set()
        for items, _ in self.staged[:count]:
            self._batch(items, now, deferred=True, window_created=window_created)
        self.staged, self.records = [], 0

    def _final_unlink(self, label, identity, now, deferred, window_created):
        """Drop the last link of a batch member: a staged one reaches the reserved directory."""
        if deferred and identity not in window_created:
            # ADR-066: a window unlinks a committed final link into object 2.
            self.objects[identity].update(links=1, changed=now)
            self.orphaned[label] = identity
        else:
            del self.objects[identity]

    def _batch(self, items, now, deferred=False, window_created=None):
        """One atomic transaction: every member publishes one generation."""
        if not items:
            raise ValueError("model batch is empty")
        self.generation += 1
        for item in items:
            kind = item["op"]
            if kind == "create":
                self._insert(item)
                data = bytearray.fromhex(item["data"])
                identity = self.next_object
                self.next_object += 1
                self.objects[identity] = {"kind": "file", "links": 1, "protection": 0, "policy": False,
                    "created": now, "modified": now, "changed": now, "generation": self.generation,
                    "data": data, "shared": set(),
                    "blocks": {block: False for block in range(-(-len(data) // BLOCK))}}
                self.used.add(item["label"])
                self.batched.add(item["label"])
                if window_created is not None:
                    window_created.add(identity)
                self.names[item["label"]] = {"object": identity, "parent": item["parent"],
                                             "name": item["name"]}
                self._touch(item["parent"], now)
                continue
            node = self._live(item["label"], "file")
            entry = self.names[item["label"]]
            if kind == "delete":
                self._touch(entry["parent"], now)
                node["links"] -= 1
                if node["links"]:
                    node["changed"] = now
                else:
                    self._final_unlink(item["label"], entry["object"], now, deferred, window_created)
                del self.names[item["label"]]
                continue
            self._directory(item["parent"])
            if kind == "rename":
                self._vacant(item["parent"], item["name"])
            else:
                target = self.names[item["victim"]]
                if (target["parent"], target["name"]) != (item["parent"], item["name"]):
                    raise ValueError("model victim does not name the replaced entry")
                if target["object"] == entry["object"]:
                    raise ValueError("model replacement requires distinct objects")
                victim = self.objects[target["object"]]
                victim["links"] -= 1
                if victim["links"]:
                    victim["changed"] = now
                else:
                    self._final_unlink(item["victim"], target["object"], now, deferred, window_created)
                del self.names[item["victim"]]
            node["changed"] = now
            self._touch(entry["parent"], now)
            self._touch(item["parent"], now)
            entry.update(parent=item["parent"], name=item["name"])

    def orphan_state(self):
        """Reserved-directory entry count and the sizes it still names."""
        return {"count": len(self.orphaned),
                "bytes": sum(len(self.objects[identity]["data"]) for identity in self.orphaned.values())}

    def _snapshot(self, kind, label):
        registered = self.views.get(label) is not None
        if kind == "snapshot_create":
            if self.maintained:
                raise ValueError("model captures no view after a maintenance step")
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
        kind = node["kind"]
        size = len(node["data"]) if kind == "file" else len(node["target"].encode()) if kind == "symlink" else 0
        if identity == 1:
            allocated = BLOCK
        elif kind == "file":
            if self.single_block and not set(node["blocks"]) <= {0}:
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
            allocation = coverage(node["blocks"]) if node["kind"] == "file" else []
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
                entry.update(alias=first.setdefault(identity, index), policy=node["policy"],
                             data=node["data"].hex(), alloc=coverage(node["blocks"]))
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




class Sequence:
    """Shared version-9 scaffolding: independent model, emitted operations, names."""
    def __init__(self, family, seed):
        self.random = Random(seed)
        self.model = ObjectModel(orphan_extents=FAMILY_ORPHAN_EXTENTS.get(family, 1))
        self.operations = []
        self.serial = 0

    def emit(self, **op):
        self.model.apply(op, len(self.operations))
        self.operations.append(op)

    def fresh(self, prefix="n"):
        self.serial += 1
        return f"{prefix}{self.serial}", f"{self.serial:04d}-{prefix}-" + prefix * 20

    def files(self):
        return self.model.labels("file")

    def directories(self):
        return ["root"] + self.model.labels("directory")

    def entry(self, label):
        return self.model.names[label]["parent"], self.model.names[label]["name"]


def replacements(model, files):
    """Source/victim pairs that name distinct live file objects."""
    return [(source, victim) for source in files for victim in files
            if source != victim and model.names[source]["object"] != model.names[victim]["object"]]


def orphanable(model, files):
    """Final links whose layout one budgeted cleanup step can empty."""
    result = []
    for label in files:
        node = model.objects[model.names[label]["object"]]
        if node["links"] != 1:
            continue
        if len(coverage(node["blocks"])) > model.orphan_extents:
            continue
        result.append(label)
    return result


def generate_replace(seed, steps):
    sequence = Sequence("replace", seed)
    model, random, emit = sequence.model, sequence.random, sequence.emit
    # Ladder: plain replacement, a hard-linked victim that survives, a
    # cross-directory replacement and a replaced file read after remount.
    emit(op="mkdir", label="d", parent="root", name="dir")
    emit(op="create", label="f", parent="root", name="source", data=payload(random, 5000))
    emit(op="create", label="g", parent="root", name="target", data=payload(random, 33))
    emit(op="rename_replace", label="f", victim="g", parent="root", name="target")
    emit(op="create", label="h", parent="d", name="kept", data=payload(random, 7))
    emit(op="link", label="h2", source="h", parent="root", name="alias")
    emit(op="create", label="v", parent="d", name="victim", data=payload(random, 1))
    emit(op="rename_replace", label="v", victim="h2", parent="root", name="alias")
    emit(op="create", label="p", parent="d", name="moved", data=payload(random, 4097))
    emit(op="rename_replace", label="p", victim="h", parent="d", name="kept")
    emit(op="write", label="p", offset=0, data="ff")
    emit(op="sync")
    emit(op="remount")
    for _ in range(4):
        label, name = sequence.fresh("k")
        emit(op="create", label=label, parent="root", name=name, data=payload(random, random.pick((0, 1, 33))))
    while len(sequence.operations) < steps:
        files, directories = sequence.files(), sequence.directories()
        empty = [d for d in directories[1:] if not model.children(d)]
        pairs = replacements(model, files)
        choices = ["sync", "remount"]
        if len(model.names) <= 40:
            choices += ["create", "create"]
            if len(directories) < 6: choices.append("mkdir")
            if files: choices.append("link")
        if files: choices += ["write", "truncate", "rename", "unlink"]
        if pairs: choices += ["rename_replace"] * 4
        if empty: choices.append("rmdir")
        kind = random.pick(choices)
        if kind in ("create", "mkdir"):
            label, name = sequence.fresh("k")
            emit(op=kind, label=label, parent=random.pick(directories), name=name,
                 **({"data": payload(random, random.pick((0, 1, 33, 5000)))} if kind == "create" else {}))
        elif kind == "link":
            label, name = sequence.fresh("k")
            emit(op=kind, label=label, source=random.pick(files), parent=random.pick(directories), name=name)
        elif kind == "write":
            offset = random.pick((0, 1, 4095, 4096))
            emit(op=kind, label=random.pick(files), offset=offset,
                 data=payload(random, random.pick(tuple(c for c in (1, 7, 300) if offset + c <= MAX_FILE_BYTES))))
        elif kind == "truncate":
            label = random.pick(files)
            current = len(model.file_data(label))
            emit(op=kind, label=label,
                 size=random.pick(tuple(s for s in (0, 1, 4095, 4096, 8193) if s != current)))
        elif kind == "rename":
            emit(op=kind, label=random.pick(files), parent=random.pick(directories), name=sequence.fresh("k")[1])
        elif kind == "rename_replace":
            source, victim = random.pick(pairs)
            parent, name = sequence.entry(victim)
            emit(op=kind, label=source, victim=victim, parent=parent, name=name)
        elif kind in ("unlink", "rmdir"):
            emit(op=kind, label=random.pick(files if kind == "unlink" else empty))
        else:
            emit(op=kind)
    return sequence.operations


def generate_orphan(seed, steps):
    sequence = Sequence("orphan", seed)
    model, random, emit = sequence.model, sequence.random, sequence.emit
    # Ladder: a data step and an object step, an empty orphan removed in one
    # step, an orphaned replacement victim and orphan state across a remount.
    emit(op="mkdir", label="d", parent="root", name="dir")
    emit(op="create", label="f", parent="d", name="file", data=payload(random, 5000))
    emit(op="orphan_file", label="f")
    emit(op="cleanup_orphan", label="f")
    emit(op="create", label="e", parent="root", name="empty", data="")
    emit(op="orphan_file", label="e")
    emit(op="cleanup_orphan", label="e")
    emit(op="create", label="g", parent="root", name="target", data=payload(random, 33))
    emit(op="create", label="v", parent="d", name="source", data=payload(random, 7))
    emit(op="rename_replace_orphan", label="v", victim="g", parent="root", name="target")
    emit(op="remount")
    emit(op="cleanup_orphan", label="g")
    emit(op="create", label="h", parent="root", name="held", data=payload(random, 4096))
    # A hole gives the orphan two extent records, at the cleanup budget.
    emit(op="write", label="h", offset=8191, data=payload(random, 7))
    emit(op="orphan_file", label="h")
    emit(op="sync")
    emit(op="remount")
    emit(op="cleanup_orphan", label="h")
    emit(op="create", label="r", parent="root", name="retained", data=payload(random, 33))
    emit(op="orphan_file", label="r")
    for _ in range(4):
        label, name = sequence.fresh("k")
        emit(op="create", label=label, parent="root", name=name, data=payload(random, random.pick((0, 1, 33))))
    while len(sequence.operations) < steps:
        files, directories = sequence.files(), sequence.directories()
        victims = orphanable(model, files)
        pairs = [(s, v) for s, v in replacements(model, files) if v in victims]
        pending = sorted(model.orphaned)
        choices = ["sync", "remount"]
        if len(model.names) <= 40:
            choices += ["create", "create"]
            if len(directories) < 6: choices.append("mkdir")
        if files: choices += ["write", "truncate", "unlink", "rename"]
        if victims: choices += ["orphan_file"] * 3
        if pairs: choices += ["rename_replace_orphan"] * 2
        if pending: choices += ["cleanup_orphan"] * 4
        kind = random.pick(choices)
        if kind in ("create", "mkdir"):
            label, name = sequence.fresh("k")
            emit(op=kind, label=label, parent=random.pick(directories), name=name,
                 **({"data": payload(random, random.pick((0, 1, 33, 5000)))} if kind == "create" else {}))
        elif kind == "write":
            # Contiguous growth keeps an orphan inside the budgeted extent model.
            label = random.pick(files)
            size = len(model.file_data(label))
            offset = random.pick(tuple(o for o in (0, 1, size) if o <= size)) if size else 0
            emit(op=kind, label=label, offset=offset,
                 data=payload(random, random.pick(tuple(c for c in (1, 7, 300) if offset + c <= MAX_FILE_BYTES))))
        elif kind == "truncate":
            label = random.pick(files)
            current = len(model.file_data(label))
            emit(op=kind, label=label, size=random.pick(tuple(s for s in (0, 1, 4095, 4096) if s != current)))
        elif kind == "rename":
            emit(op=kind, label=random.pick(files), parent=random.pick(directories), name=sequence.fresh("k")[1])
        elif kind == "rename_replace_orphan":
            source, victim = random.pick(pairs)
            parent, name = sequence.entry(victim)
            emit(op=kind, label=source, victim=victim, parent=parent, name=name)
        elif kind in ("orphan_file", "cleanup_orphan"):
            emit(op=kind, label=random.pick(victims if kind == "orphan_file" else pending))
        elif kind == "unlink":
            emit(op=kind, label=random.pick(files))
        else:
            emit(op=kind)
    return sequence.operations


def generate_space(seed, steps):
    sequence = Sequence("space", seed)
    model, random, emit = sequence.model, sequence.random, sequence.emit
    # Ladder: a reservation past the written blocks, a write that consumes one
    # reserved block, a tightly bounded reservation, the persistent policy in
    # both directions and an archived metadata restore.
    emit(op="mkdir", label="d", parent="root", name="dir")
    emit(op="create", label="f", parent="d", name="file", data=payload(random, 4097))
    emit(op="preallocate", label="f", offset=8192, length=8192)
    emit(op="write", label="f", offset=8192, data=payload(random, 7))
    emit(op="preallocate_bounded", label="f", offset=0, length=1, max_blocks=1, max_records=4096)
    emit(op="set_data_policy", label="f", policy=True)
    emit(op="write", label="f", offset=0, data="ff")
    emit(op="set_data_policy", label="f", policy=False)
    emit(op="create", label="g", parent="root", name="kept", data=payload(random, 33))
    emit(op="set_data_policy", label="g", policy=True)
    emit(op="restore_metadata", label="g", protection=5, created=1, modified=2, changed=3)
    emit(op="restore_metadata", label="d", protection=493, created=4, modified=5, changed=6)
    emit(op="preallocate", label="g", offset=4096, length=4096)
    emit(op="truncate", label="g", size=1)
    emit(op="sync")
    emit(op="remount")
    for _ in range(4):
        label, name = sequence.fresh("k")
        emit(op="create", label=label, parent="root", name=name, data=payload(random, random.pick((0, 1, 33))))
    while len(sequence.operations) < steps:
        files, directories = sequence.files(), sequence.directories()
        empty = [d for d in directories[1:] if not model.children(d)]
        choices = ["sync", "remount"]
        if len(model.names) <= 40:
            choices += ["create", "create"]
            if len(directories) < 6: choices.append("mkdir")
        if files: choices += ["write", "truncate", "unlink", "rename", "set_protection", "restore_metadata"]
        if files: choices += ["preallocate"] * 3 + ["preallocate_bounded"] * 2 + ["set_data_policy"] * 2
        if empty: choices.append("rmdir")
        kind = random.pick(choices)
        if kind in ("create", "mkdir"):
            label, name = sequence.fresh("k")
            emit(op=kind, label=label, parent=random.pick(directories), name=name,
                 **({"data": payload(random, random.pick((0, 1, 33, 5000)))} if kind == "create" else {}))
        elif kind == "write":
            offset = random.pick((0, 1, 4095, 4096, 8191))
            emit(op=kind, label=random.pick(files), offset=offset,
                 data=payload(random, random.pick(tuple(c for c in (1, 7, 300) if offset + c <= MAX_FILE_BYTES))))
        elif kind == "truncate":
            label = random.pick(files)
            current = len(model.file_data(label))
            emit(op=kind, label=label,
                 size=random.pick(tuple(s for s in (0, 1, 4095, 4096, 8193) if s != current)))
        elif kind in ("preallocate", "preallocate_bounded"):
            label = random.pick(files)
            offset = random.pick((0, 4096, 8192, 12288))
            length = random.pick((1, 4096, 8192))
            operation = {"op": kind, "label": label, "offset": offset, "length": length}
            if kind == "preallocate_bounded":
                # The block budget is exact. A record budget counts stored
                # extents, whose boundaries follow physical placement, so the
                # generator uses the admission maximum for it.
                first, last = offset // BLOCK, -(-(offset + length) // BLOCK)
                operation.update(max_blocks=last - first, max_records=4096)
            emit(**operation)
        elif kind == "set_data_policy":
            label = random.pick(files)
            emit(op=kind, label=label,
                 policy=not model.objects[model.names[label]["object"]]["policy"])
        elif kind == "restore_metadata":
            label = random.pick(files + directories[1:])
            emit(op=kind, label=label, protection=random.pick(PROTECTIONS),
                 created=random.pick((1, 2, 3)), modified=random.pick((4, 5)), changed=random.pick((6, 7)))
        elif kind == "set_protection":
            emit(op=kind, label=random.pick(files), protection=random.pick(PROTECTIONS))
        elif kind == "rename":
            emit(op=kind, label=random.pick(files), parent=random.pick(directories), name=sequence.fresh("k")[1])
        elif kind in ("unlink", "rmdir"):
            emit(op=kind, label=random.pick(files if kind == "unlink" else empty))
        else:
            emit(op=kind)
    return sequence.operations


def generate_batch(seed, steps):
    sequence = Sequence("batch", seed)
    model, random, emit = sequence.model, sequence.random, sequence.emit
    # Ladder: a create group, a mixed move/delete group, an atomic replacement
    # inside a group, a staged window group and its acknowledged prefix.
    emit(op="mkdir", label="d", parent="root", name="dir")
    emit(op="batch", items=[{"op": "create", "label": "a", "parent": "root", "name": "a",
                             "data": payload(random, 4097)},
                            {"op": "create", "label": "b", "parent": "d", "name": "b",
                             "data": payload(random, 33)}])
    emit(op="batch", items=[{"op": "rename", "label": "a", "parent": "d", "name": "moved"},
                            {"op": "create", "label": "c", "parent": "root", "name": "c",
                             "data": payload(random, 7)}])
    emit(op="batch", items=[{"op": "replace", "label": "b", "victim": "a", "parent": "d",
                             "name": "moved"},
                            {"op": "delete", "label": "c"}])
    emit(op="window_batch", items=[{"op": "create", "label": "w", "parent": "root", "name": "w",
                                    "data": payload(random, 1)}])
    emit(op="window_fsync")
    emit(op="window_batch", items=[{"op": "rename", "label": "w", "parent": "d", "name": "w2"}])
    emit(op="window_commit")
    # Staged final unlinks: an acknowledged group publishes its orphan at a
    # remount, an unacknowledged group leaves its file in the namespace, a
    # commit publishes the rest, a staged create deleted inside its own window
    # leaves nothing, and a staged replacement orphans a final-link victim.
    emit(op="create", label="o", parent="root", name="reserved", data=payload(random, 4097))
    emit(op="create", label="e", parent="root", name="kept", data=payload(random, 7))
    emit(op="create", label="g", parent="d", name="gone", data=payload(random, 33))
    emit(op="window_batch", items=[{"op": "delete", "label": "o"}])
    emit(op="window_fsync")
    emit(op="window_batch", items=[{"op": "delete", "label": "e"}])
    emit(op="remount")
    emit(op="window_batch", items=[{"op": "delete", "label": "g"}])
    emit(op="window_commit")
    emit(op="window_batch", items=[{"op": "create", "label": "t", "parent": "root", "name": "t",
                                    "data": payload(random, 5)}])
    emit(op="window_fsync")
    emit(op="window_batch", items=[{"op": "delete", "label": "t"}])
    emit(op="window_commit")
    emit(op="create", label="p", parent="root", name="p", data=payload(random, 9))
    emit(op="create", label="q", parent="root", name="q", data=payload(random, 11))
    emit(op="window_batch", items=[{"op": "replace", "label": "p", "victim": "q", "parent": "root",
                                    "name": "q"}])
    emit(op="window_commit")
    emit(op="sync")
    emit(op="remount")
    for _ in range(4):
        label, name = sequence.fresh("k")
        emit(op="create", label=label, parent="root", name=name, data=payload(random, random.pick((0, 1, 33))))
    # A label a window group names is spent: the acknowledged prefix decides
    # where its object ends up, so a later operation never names it again.
    spent = {"w", "o", "e", "g", "t", "p", "q"}

    def group(taken):
        """One bounded group of members over labels no other member claims."""
        items = []
        for _ in range(1 + random.next() % 4):
            live = [label for label in sequence.files() if label not in taken]
            pairs = [(s, v) for s, v in replacements(model, live) if s not in taken and v not in taken]
            members = ["create"]
            if live: members += ["rename", "delete"]
            if pairs: members.append("replace")
            member = random.pick(members)
            if member == "create":
                label, name = sequence.fresh("k")
                items.append({"op": "create", "label": label, "parent": random.pick(directories),
                              "name": name, "data": payload(random, random.pick((0, 1, 33)))})
                taken.add(label)
                continue
            if member == "replace":
                source, victim = random.pick(pairs)
                parent, name = sequence.entry(victim)
                items.append({"op": "replace", "label": source, "victim": victim,
                              "parent": parent, "name": name})
                taken.update((source, victim))
                continue
            label = random.pick(live)
            taken.add(label)
            if member == "delete":
                items.append({"op": "delete", "label": label})
            else:
                items.append({"op": "rename", "label": label, "parent": random.pick(directories),
                              "name": sequence.fresh("k")[1]})
        return items

    while len(sequence.operations) < steps:
        directories = sequence.directories()
        files = [label for label in sequence.files() if label not in spent]
        empty = [d for d in directories[1:] if not model.children(d)]
        unacknowledged = sum(len(items) for items, done in model.staged if not done)
        if model.staged:
            # A window admits staged groups, its acknowledging fsync, a commit
            # and a remount; a direct operation belongs outside it.
            choices = ["window_commit", "remount"]
            if len(model.staged) < 6 and unacknowledged < 5: choices += ["window_batch"] * 2
            if model.records < 6 or not unacknowledged:
                choices.append("window_fsync")
            kind = random.pick(choices)
        else:
            choices = ["sync", "remount", "batch", "batch", "batch", "window_batch"]
            if len(model.names) <= 40:
                choices.append("create")
                if len(directories) < 6: choices.append("mkdir")
            if files: choices += ["write", "truncate", "unlink"]
            if empty: choices.append("rmdir")
            kind = random.pick(choices)
        if kind in ("batch", "window_batch"):
            items = group(set(spent))
            if kind == "window_batch":
                spent.update(item["label"] for item in items)
                spent.update(item["victim"] for item in items if item["op"] == "replace")
            emit(op=kind, items=items)
        elif kind in ("create", "mkdir"):
            label, name = sequence.fresh("k")
            emit(op=kind, label=label, parent=random.pick(directories), name=name,
                 **({"data": payload(random, random.pick((0, 1, 33)))} if kind == "create" else {}))
        elif kind == "write":
            offset = random.pick((0, 1, 4095, 4096))
            emit(op=kind, label=random.pick(files), offset=offset,
                 data=payload(random, random.pick(tuple(c for c in (1, 7, 300) if offset + c <= MAX_FILE_BYTES))))
        elif kind == "truncate":
            label = random.pick(files)
            current = len(model.file_data(label))
            emit(op=kind, label=label, size=random.pick(tuple(s for s in (0, 1, 4095, 4096) if s != current)))
        elif kind in ("unlink", "rmdir"):
            emit(op=kind, label=random.pick(files if kind == "unlink" else empty))
        else:
            emit(op=kind)
    return sequence.operations


def generate_maintenance(seed, steps):
    sequence = Sequence("maintenance", seed)
    model, random, emit = sequence.model, sequence.random, sequence.emit
    # Ladder: every view is captured before the first maintenance step, then
    # namespace churn produces reclaimable capacity and the steps drain it.
    emit(op="mkdir", label="d", parent="root", name="dir")
    emit(op="create", label="f", parent="d", name="file", data=payload(random, 5000))
    emit(op="snapshot_create", label="s1")
    emit(op="snapshot_open", label="s1")
    emit(op="write", label="f", offset=0, data=payload(random, 7))
    emit(op="snapshot_create", label="s2")
    emit(op="snapshot_inspect", label="s1")
    emit(op="snapshot_close", label="s1")
    emit(op="create", label="g", parent="root", name="garbage", data=payload(random, 4097))
    emit(op="unlink", label="g")
    emit(op="truncate", label="f", size=1)
    for _ in range(3):
        emit(op="reclaim_step")
    for _ in range(3):
        emit(op="snapshot_maintenance_step")
    emit(op="remount")
    emit(op="reclaim_step")
    emit(op="snapshot_maintenance_step")
    emit(op="snapshot_open", label="s2")
    emit(op="snapshot_inspect", label="s2")
    emit(op="snapshot_close", label="s2")
    emit(op="snapshot_delete", label="s1")
    emit(op="reclaim_step")
    emit(op="snapshot_maintenance_step")
    emit(op="sync")
    emit(op="remount")
    for _ in range(4):
        label, name = sequence.fresh("k")
        emit(op="create", label=label, parent="root", name=name, data=payload(random, random.pick((0, 1, 33))))
    while len(sequence.operations) < steps:
        files, directories = sequence.files(), sequence.directories()
        empty = [d for d in directories[1:] if not model.children(d)]
        live = sorted(label for label, view in model.views.items() if view is not None)
        # The ladder view survives every case, so a captured oracle always applies.
        closed = [label for label in live if label not in model.handles and label != "s2"]
        opened = sorted(model.handles)
        choices = ["sync", "remount"] + ["reclaim_step"] * 4 + ["snapshot_maintenance_step"] * 4
        if len(model.names) <= 24:
            choices.append("create")
            if len(directories) < 5: choices.append("mkdir")
        if files: choices += ["write", "truncate", "unlink"]
        if empty: choices.append("rmdir")
        if closed: choices += ["snapshot_open", "snapshot_delete"]
        if opened: choices += ["snapshot_inspect", "snapshot_close"]
        kind = random.pick(choices)
        if kind in ("create", "mkdir"):
            label, name = sequence.fresh("k")
            emit(op=kind, label=label, parent=random.pick(directories), name=name,
                 **({"data": payload(random, random.pick((0, 1, 33, 4097)))} if kind == "create" else {}))
        elif kind == "write":
            offset = random.pick((0, 1, 4095, 4096))
            emit(op=kind, label=random.pick(files), offset=offset,
                 data=payload(random, random.pick(tuple(c for c in (1, 7, 300) if offset + c <= MAX_FILE_BYTES))))
        elif kind == "truncate":
            label = random.pick(files)
            current = len(model.file_data(label))
            emit(op=kind, label=label, size=random.pick(tuple(s for s in (0, 1, 4096, 8193) if s != current)))
        elif kind in ("unlink", "rmdir"):
            emit(op=kind, label=random.pick(files if kind == "unlink" else empty))
        elif kind in ("snapshot_open", "snapshot_delete"):
            emit(op=kind, label=random.pick(closed))
        elif kind in ("snapshot_inspect", "snapshot_close"):
            emit(op=kind, label=random.pick(opened))
        else:
            emit(op=kind)
    return sequence.operations


def generate_captured(seed, steps):
    sequence = Sequence("captured", seed)
    model, random, emit = sequence.model, sequence.random, sequence.emit
    # Ladder: a view over a multi-block file, a hard link, a symlink, a clone
    # and a reservation, then independent change of each after the capture.
    emit(op="mkdir", label="a", parent="root", name="alpha")
    emit(op="create", label="f", parent="a", name="file", data=payload(random, 5000))
    emit(op="link", label="l", source="f", parent="root", name="alias")
    emit(op="symlink", label="s", parent="root", name="sym", target="alpha/file")
    emit(op="clone_file", label="c", source="f", parent="root", name="clone")
    emit(op="preallocate", label="f", offset=8192, length=8192)
    # A whole-block CloneRange marks the source layout and carries the source
    # change time; the destination takes the shared blocks and a private copy
    # of each partial boundary block.
    emit(op="create", label="g", parent="a", name="range", data=payload(random, 8200))
    emit(op="clone_range", source="g", source_offset=0, destination="f", destination_offset=0,
         length=8192)
    emit(op="snapshot_create", label="v1")
    emit(op="snapshot_open", label="v1")
    emit(op="snapshot_inspect", label="v1")
    emit(op="write", label="c", offset=4096, data=payload(random, 7))
    emit(op="truncate", label="f", size=4096)
    emit(op="unlink", label="l")
    emit(op="set_protection", label="f", protection=5)
    # A second call over an already shared range leaves the source layout and
    # its change time alone; a boundary-only call shares no complete block.
    emit(op="create", label="h", parent="root", name="range2", data=payload(random, 1))
    emit(op="clone_range", source="g", source_offset=0, destination="h", destination_offset=0,
         length=8192)
    emit(op="clone_range", source="g", source_offset=1, destination="h", destination_offset=4097,
         length=100)
    emit(op="snapshot_create", label="v2")
    emit(op="snapshot_close", label="v1")
    emit(op="remount")
    emit(op="snapshot_open", label="v2")
    emit(op="snapshot_inspect", label="v2")
    emit(op="snapshot_close", label="v2")
    emit(op="snapshot_delete", label="v1")
    emit(op="sync")
    emit(op="remount")
    for _ in range(4):
        label, name = sequence.fresh("k")
        emit(op="create", label=label, parent="root", name=name, data=payload(random, random.pick((0, 1, 33))))
    while len(sequence.operations) < steps:
        files, directories = sequence.files(), sequence.directories()
        symlinks = model.labels("symlink")
        shallow = [d for d in directories if len(model.path(d)) < 2]
        empty = [d for d in directories[1:] if not model.children(d)]
        live = sorted(label for label, view in model.views.items() if view is not None)
        closed = [label for label in live if label not in model.handles]
        opened = sorted(model.handles)
        pairs = [(source, destination) for source in files for destination in files
                 if model.names[source]["object"] != model.names[destination]["object"]
                 and model.file_data(source)]
        choices = ["sync", "remount"]
        if len(model.names) <= 24:
            choices += ["create", "symlink"]
            if len(directories) < 5: choices.append("mkdir")
            if files: choices += ["link", "clone_file"]
        if pairs: choices += ["clone_range"] * 2
        if files: choices += ["write", "truncate", "unlink", "rename", "set_protection", "preallocate"]
        if symlinks: choices.append("unlink_symlink")
        if empty: choices.append("rmdir")
        if len(live) < 6: choices += ["snapshot_create"] * 2
        if closed: choices += ["snapshot_open", "snapshot_delete"]
        if opened: choices += ["snapshot_inspect", "snapshot_close"]
        kind = random.pick(choices)
        if kind in ("create", "mkdir", "symlink"):
            label, name = sequence.fresh("k")
            emit(op=kind, label=label, parent=random.pick(shallow if kind == "mkdir" else directories),
                 name=name, **({"data": payload(random, random.pick((0, 1, 33, 5000)))} if kind == "create"
                               else {"target": random.pick(SYMLINK_TARGETS)} if kind == "symlink" else {}))
        elif kind in ("link", "clone_file"):
            label, name = sequence.fresh("k")
            emit(op=kind, label=label, source=random.pick(files), parent=random.pick(directories), name=name)
        elif kind == "write":
            offset = random.pick((0, 1, 4095, 4096))
            emit(op=kind, label=random.pick(files), offset=offset,
                 data=payload(random, random.pick(tuple(c for c in (1, 7, 300) if offset + c <= MAX_FILE_BYTES))))
        elif kind == "truncate":
            label = random.pick(files)
            current = len(model.file_data(label))
            emit(op=kind, label=label, size=random.pick(tuple(s for s in (0, 1, 4096, 8193) if s != current)))
        elif kind == "clone_range":
            source, destination = random.pick(pairs)
            size = len(model.file_data(source))
            start = random.pick(tuple(o for o in (0, 1, 4095, 4096, 4097) if o < size))
            target = random.pick((start % BLOCK, start % BLOCK + BLOCK))
            lengths = tuple(n for n in (1, 7, 100, 4096, 5000) if start + n <= size and target + n <= MAX_FILE_BYTES)
            emit(op=kind, source=source, source_offset=start, destination=destination,
                 destination_offset=target,
                 length=random.pick(lengths) if lengths else min(size - start, MAX_FILE_BYTES - target))
        elif kind == "preallocate":
            emit(op=kind, label=random.pick(files), offset=random.pick((0, 4096, 8192)),
                 length=random.pick((1, 4096, 8192)))
        elif kind == "set_protection":
            emit(op=kind, label=random.pick(files), protection=random.pick(PROTECTIONS))
        elif kind == "rename":
            emit(op=kind, label=random.pick(files), parent=random.pick(directories), name=sequence.fresh("k")[1])
        elif kind in ("unlink", "unlink_symlink", "rmdir"):
            emit(op=kind, label=random.pick(files if kind == "unlink"
                                            else symlinks if kind == "unlink_symlink" else empty))
        elif kind == "snapshot_create":
            emit(op=kind, label=sequence.fresh("s")[0])
        elif kind in ("snapshot_open", "snapshot_delete"):
            emit(op=kind, label=random.pick(closed))
        elif kind in ("snapshot_inspect", "snapshot_close"):
            emit(op=kind, label=random.pick(opened))
        else:
            emit(op=kind)
    return sequence.operations


GENERATORS = {"window": generate_window, "snapshot": generate_snapshot, "namespace": generate_namespace,
              "replace": generate_replace, "orphan": generate_orphan, "space": generate_space,
              "batch": generate_batch, "maintenance": generate_maintenance,
              "captured": generate_captured}


def generate_family(family, seed, steps):
    if family not in FAMILY_VERSIONS:
        raise ValueError("unknown operation family")
    runner.scenario.integer(seed, 0, MASK)
    runner.scenario.integer(steps, 64, 256)
    return GENERATORS[family](seed, steps)


def family_case(family, seed, steps, prefix, pages):
    """Scenario plus the independent model that computed its exact expected state."""
    runner.scenario.integer(prefix, 1, steps)
    version = FAMILY_VERSIONS[family]
    operations = generate_family(family, seed, steps)[:prefix] + [{"op": "remount"}]
    value = {"version": version,
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
        model = ObjectModel(single_block=family == "snapshot",
                            orphan_extents=FAMILY_ORPHAN_EXTENTS.get(family, 1))
        for index, op in enumerate(operations):
            model.apply(op, index)
        value.update(expected=model.entries() if family == "snapshot" else model.linked(),
                     snapshot_limits=dict(SNAPSHOT_LIMITS), expected_snapshots=model.snapshots())
        if version >= 9:
            value.update(data_policy=FAMILY_POLICY.get(family, False),
                         orphan_extents=FAMILY_ORPHAN_EXTENTS.get(family, 1),
                         expected_orphans=model.orphan_state())
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
    if control in ("maintenance-entry", "captured-coverage"):
        for view in value["expected_snapshots"]:
            if control == "maintenance-entry":
                entry = first(view["entries"], lambda e: e["metadata"]["kind"] == "file" and e["data"])
                if entry is not None:
                    entry["data"] = flip(entry["data"])
                    return [str(view["id"])] + entry["path"]
            else:
                entry = first(view["entries"], lambda e: e["allocation"])
                if entry is not None:
                    entry["allocation"][-1]["length"] += BLOCK
                    return [str(view["id"])] + entry["path"]
        return None
    if control == "clone-changed":
        for view in value["expected_snapshots"]:
            entry = first(view["entries"],
                          lambda e: e["metadata"]["object_id"] in model.clone_sources)
            if entry is not None:
                entry["metadata"]["changed"] = [entry["metadata"]["changed"][0] + 1, 0]
                return [str(view["id"])] + entry["path"]
        return None
    if control == "reservation":
        entry = first(expected, lambda e: e["kind"] == "file" and e.get("alloc"))
        if entry is None:
            return None
        entry["alloc"][-1]["length"] += BLOCK
        return entry["path"]
    if control == "policy-flag":
        entry = first(expected, lambda e: e["kind"] == "file")
        if entry is None:
            return None
        entry["policy"] = not entry["policy"]
        return entry["path"]
    if control == "batch-path":
        paths = [model.path(label) for label in sorted(model.batched) if label in model.names]
        entry = first(expected, lambda e: e["path"] in paths)
        if entry is None:
            return None
        location = entry["path"]
        entry["path"] = location[:-1] + [location[-1] + "~"]
        return location
    if control in ("orphan-count", "orphan-bytes"):
        orphans = value["expected_orphans"]
        if not orphans["count"] and not orphans["bytes"]:
            return None
        key = "count" if control == "orphan-count" else "bytes"
        orphans[key] = orphans[key] - 1 if orphans[key] else 1
        return ["orphans", key, str(orphans[key])]
    if control == "replaced-byte":
        paths = [model.path(label) for label in sorted(model.replaced) if label in model.names]
        entry = first(expected, lambda e: e["kind"] == "file" and e["data"] and e["path"] in paths)
        if entry is None:
            return None
        entry["data"] = flip(entry["data"])
        return entry["path"]
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
