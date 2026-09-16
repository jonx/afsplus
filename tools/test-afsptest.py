#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""Fresh-process semantic bundle and independently decoded trace regressions."""
import importlib.util
import json
import io
import struct
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("afsptest", Path(__file__).with_name("afsptest.py"))
tool = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tool)
BINARY = tool.ROOT / "target/debug/afsplus-scenario"


def fixture():
    return {"version": 1, "volume": {"block_size": 4096, "blocks": 256, "region_size": 64, "log_slots": 8},
        "operations": [{"op": "create", "label": "f", "parent": "root", "name": "café", "data": "00ff"},
            {"op": "remount"}, {"op": "write", "label": "f", "offset": 1, "data": "42"},
            {"op": "sync"}, {"op": "remount"}],
        "expected": [{"path": ["café"], "kind": "file", "data": "0042"}]}


def snapshot_fixture(pages=2):
    # mkfs starts at generation/transaction 1 and object ID 16. Creating
    # one file publishes generation 2; snapshot_create captures that checkpoint.
    root = {"object_id": 1, "kind": "directory", "size": 0, "allocated": 4096,
            "links": 1, "protection": 0, "created": [0, 0], "modified": [1, 0],
            "changed": [1, 0], "content_generation": 2}
    file = dict(root, object_id=16, kind="file", size=1, created=[1, 0])
    return {"version": 7, "volume": {"block_size": 4096, "blocks": 512,
            "region_size": 64, "log_slots": 8, "tree_cache_pages": pages},
        "flight_capacity": 256, "flight_categories": 127, "flight_sink": None,
        "snapshot_limits": {"max_edit_records": 4096, "max_views": 16, "reclaim_records": 8},
        "operations": [{"op": "create", "label": "f", "parent": "root", "name": "file", "data": "61"},
            {"op": "snapshot_create", "label": "s"}, {"op": "snapshot_open", "label": "s"},
            {"op": "write", "label": "f", "offset": 0, "data": "62"},
            {"op": "snapshot_inspect", "label": "s"}, {"op": "remount"},
            {"op": "snapshot_open", "label": "s"}, {"op": "snapshot_inspect", "label": "s"}],
        "expected": [{"path": ["file"], "kind": "file", "data": "62"}],
        "expected_snapshots": [{"id": 1, "generation": 2, "committed_tx_id": 2, "root": root,
            "entries": [{"path": ["file"], "metadata": file, "data": "61",
                         "allocation": [{"offset": 0, "length": 4096, "unwritten": False}]}]}]}


def linked_fixture(pages=2):
    # Hand-computed state: aliases share bytes and protection, the clone keeps
    # its own bytes, and the unaligned range copies the alias bytes 4095..4096.
    shared = "00" * 4095 + "a1ff"
    cover = [{"offset": 0, "length": 8192, "unwritten": False}]
    return {"version": 9, "volume": {"block_size": 4096, "blocks": 512, "region_size": 64,
            "log_slots": 8, "tree_cache_pages": pages},
        "flight_capacity": 32, "flight_categories": 127, "flight_sink": None,
        "snapshot_limits": {"max_edit_records": 4096, "max_views": 16, "reclaim_records": 8},
        "data_policy": True, "orphan_extents": 1,
        "operations": [
            {"op": "mkdir", "label": "d", "parent": "root", "name": "dir"},
            {"op": "create", "label": "f", "parent": "d", "name": "file", "data": "00" * 4095 + "a1b2"},
            {"op": "link", "label": "l", "source": "f", "parent": "root", "name": "alias"},
            {"op": "write", "label": "l", "offset": 0, "data": "42"},
            {"op": "symlink", "label": "s", "parent": "d", "name": "link", "target": "../ταξί"},
            {"op": "set_protection", "label": "f", "protection": 7},
            {"op": "clone_file", "label": "c", "source": "f", "parent": "root", "name": "clone"},
            {"op": "write", "label": "f", "offset": 4096, "data": "ff"},
            {"op": "clone_range", "source": "l", "source_offset": 4095, "destination": "c",
             "destination_offset": 4095, "length": 2},
            {"op": "write", "label": "f", "offset": 0, "data": "00"},
            {"op": "set_data_policy", "label": "c", "policy": True},
            {"op": "rename", "label": "d", "parent": "root", "name": "moved"},
            {"op": "remount"}],
        "expected": [
            {"path": ["alias"], "kind": "file", "links": 2, "protection": 7, "alias": 0,
             "policy": False, "data": shared, "alloc": cover},
            {"path": ["clone"], "kind": "file", "links": 1, "protection": 7, "alias": 1,
             "policy": True, "data": "42" + "00" * 4094 + "a1ff", "alloc": cover},
            {"path": ["moved"], "kind": "directory", "links": 1, "protection": 0},
            {"path": ["moved", "file"], "kind": "file", "links": 2, "protection": 7, "alias": 0,
             "policy": False, "data": shared, "alloc": cover},
            {"path": ["moved", "link"], "kind": "symlink", "links": 1, "protection": 0, "target": "../ταξί"}],
        "expected_snapshots": [], "expected_orphans": {"count": 0, "bytes": 0}}



def lifecycle_fixture(pages=2, mask=32767, capacity=256, sink=None):
    """A version-8 scenario reaching the allocator, the mutable trees, reclaim,
    mount recovery, a refused mount, formatting, standalone verification,
    staged window writes and read-only view descents."""
    return {"version": 8, "volume": {"block_size": 4096, "blocks": 512, "region_size": 64,
            "log_slots": 8, "tree_cache_pages": pages},
        "flight_capacity": capacity, "flight_categories": mask, "flight_sink": sink,
        "snapshot_limits": {"max_edit_records": 4096, "max_views": 16, "reclaim_records": 8},
        "expected_snapshots": [], "expected_findings": [],
        "operations": [
            {"op": "mkdir", "label": "d", "parent": "root", "name": "dir"},
            {"op": "create", "label": "f", "parent": "d", "name": "café", "data": "00ff"},
            {"op": "window_write", "label": "f", "offset": 1, "data": "42"},
            {"op": "window_fsync"},
            {"op": "window_commit"},
            {"op": "write", "label": "f", "offset": 0, "data": "01"},
            {"op": "sync"},
            {"op": "truncate", "label": "f", "size": 1},
            {"op": "verify"},
            {"op": "remount_refused"},
            {"op": "remount"}],
        "expected": [{"path": ["dir"], "kind": "directory"},
                     {"path": ["dir", "café"], "kind": "file", "data": "01"}]}



def lifecycle_value(ops, *, blocks=256, region=64, pages=2, capacity=256, mask=32767,
                    expected=None, findings=None):
    return {"version": 8, "volume": {"block_size": 4096, "blocks": blocks,
            "region_size": region, "log_slots": 8, "tree_cache_pages": pages},
        "flight_capacity": capacity, "flight_categories": mask, "flight_sink": None,
        "snapshot_limits": {"max_edit_records": 4096, "max_views": 16, "reclaim_records": 8},
        "expected_snapshots": [], "expected_findings": findings or [],
        "operations": ops, "expected": expected or []}


def lifecycle_scenarios():
    """One version-8 scenario per group of diagnostic kinds.

    The block addresses of the image edits belong to this exact geometry and
    operation prefix; the runner formats the same image for the same input.
    """
    big = "ab" * 3000
    seeded = [{"op": "create", "label": "f", "parent": "root", "name": "a", "data": big},
              {"op": "sync"}]
    window = [{"op": "create", "label": "f", "parent": "root", "name": "a", "data": "0102"},
              {"op": "window_write", "label": "f", "offset": 0, "data": "ab" * 2000}]
    return {
        "namespace": lifecycle_value([
            {"op": "mkdir", "label": "d", "parent": "root", "name": "dir"},
            {"op": "create", "label": "f", "parent": "d", "name": "café", "data": "00ff"},
            {"op": "window_write", "label": "f", "offset": 1, "data": "42"},
            {"op": "window_fsync"}, {"op": "window_fsync"},
            {"op": "verify"}, {"op": "window_commit"},
            {"op": "write", "label": "f", "offset": 0, "data": "01"}, {"op": "sync"},
            {"op": "truncate", "label": "f", "size": 1},
            {"op": "remount_refused"}, {"op": "remount"}], blocks=512),
        "mount-replay": lifecycle_value([
            {"op": "create", "label": "f", "parent": "root", "name": "a", "data": "0102"},
            {"op": "window_write", "label": "f", "offset": 0, "data": "ab" * 2000},
            {"op": "window_fsync"},
            {"op": "window_write", "label": "f", "offset": 0, "data": "cd" * 2000},
            {"op": "remount"}, {"op": "verify"}], blocks=512),
        "format-fault": lifecycle_value([
            {"op": "format_fault", "class": "write", "index": 0},
            {"op": "create", "label": "f", "parent": "root", "name": "a", "data": "01"},
            {"op": "sync"}], blocks=512),
        "spill": lifecycle_value([
            {"op": "create", "label": "f%d" % i, "parent": "root",
             "name": "%04d-%s" % (i, "n" * 180), "data": "01"} for i in range(60)],
            blocks=1024, region=256),
        "nospace": lifecycle_value([
            {"op": "create", "label": "g%d" % i, "parent": "root", "name": "g%d" % i,
             "data": "cd" * 4000} for i in range(40)], blocks=64),
        "reclaim-fail": lifecycle_value(seeded + [
            {"op": "fault", "class": "read", "index": 0},
            {"op": "snapshot_create", "label": "s"}, {"op": "sync"}]),
        "view-maintenance": lifecycle_value(seeded + [
            {"op": "fault", "class": "write", "index": 0},
            {"op": "snapshot_create", "label": "s"}, {"op": "sync"}]),
        "tree-io": lifecycle_value(seeded + [
            {"op": "fault", "class": "read", "index": 2},
            {"op": "snapshot_create", "label": "s"}, {"op": "sync"}]),
        "view-read": lifecycle_value(seeded + [
            {"op": "snapshot_create", "label": "s"}, {"op": "snapshot_open", "label": "s"},
            {"op": "fault", "class": "read", "index": 6},
            {"op": "snapshot_inspect", "label": "s"}, {"op": "sync"}]),
        "window-log": lifecycle_value(window + [
            {"op": "fault", "class": "write", "index": 0},
            {"op": "window_fsync"}, {"op": "window_commit"}]),
        "window-fail": lifecycle_value(window + [
            {"op": "fault", "class": "write", "index": 1},
            {"op": "window_fsync"}, {"op": "window_commit"}]),
        "data-write": lifecycle_value(seeded + [
            {"op": "fault", "class": "write", "index": 0},
            {"op": "window_write", "label": "f", "offset": 0, "data": "cd" * 3000},
            {"op": "window_fsync"}, {"op": "window_commit"}]),
        "verify-failed": lifecycle_value(seeded + [
            {"op": "corrupt", "lba": 4, "offset": 0, "byte": 255}, {"op": "verify"}]),
        "verify-finding": lifecycle_value(seeded + [
            {"op": "reseal", "lba": 27, "offset": 44, "byte": 2}, {"op": "verify"}],
            findings=[{"scope": 3, "phase": 13, "kind": 1, "region": 0, "ordinal": 0,
                       "object": 16, "block": 0}]),
        "object-missing": lifecycle_value(seeded + [
            {"op": "reseal", "lba": 30, "offset": 96, "byte": 255}, {"op": "remount"},
            {"op": "write", "label": "f", "offset": 0, "data": "cd"}, {"op": "sync"}]),
    }


# Scenarios whose run and checker are clean once their expected state is settled.
LIFECYCLE_CLEAN = ("namespace", "mount-replay", "format-fault", "spill")


def lifecycle_kinds(records):
    flight = records["flight-recorder.bin"]
    events = struct.unpack_from("<I", flight, 8)[0]
    offset, seen, size = 28, set(), 89 + tool.PAYLOAD_BYTES
    def batch(offset):
        retained = struct.unpack_from("<QQQQQQBI", flight, offset)[7]
        offset += 53
        for _ in range(retained):
            seen.add(flight[offset + 24])
            offset += size
        return offset
    offset = batch(offset)
    for _ in range(events):
        offset += 29
        offset = batch(offset)
    assert offset == len(flight)
    return seen


class ReplayTests(unittest.TestCase):
    def settle(self, value):
        records, _ = tool.execute(tool.encoded(value), BINARY)
        actual = json.loads(records["actual.json"])
        self.assertIsNone(actual["failure"])
        value = dict(value)
        value["expected"] = sorted(actual["entries"], key=lambda entry: entry["path"])
        value["expected_snapshots"] = actual["snapshots"]
        return value

    def test_v8_faults_and_image_edits_produce_every_reachable_kind(self):
        union = set()
        for name, value in lifecycle_scenarios().items():
            if name in LIFECYCLE_CLEAN:
                value = self.settle(value)
            records, success = tool.execute(tool.encoded(value), BINARY)
            self.assertEqual(records["flight-recorder.bin"][:8], b"AFSFLT06")
            union |= lifecycle_kinds(records)
            # A clean scenario passes; a fault or an edited image is a recorded
            # failure, and replay reproduces that failure byte for byte.
            self.assertEqual(success, name in LIFECYCLE_CLEAN, name)
            with tempfile.TemporaryDirectory(prefix="afsplus-lifecycle-kind-") as temporary:
                path = Path(temporary) / name
                tool.bundle.publish(path, records)
                self.assertEqual(tool.replay(path, BINARY), name in LIFECYCLE_CLEAN, name)
        # ApiUnwound alone has no host scenario: the API guard reports it only
        # when a panic unwinds through it.
        self.assertEqual(sorted(set(range(1, 65)) - union), [11])

    def test_v8_expected_findings_are_compared_exactly(self):
        value = lifecycle_scenarios()["verify-finding"]
        records, success = tool.execute(tool.encoded(value), BINARY)
        self.assertFalse(success)
        self.assertEqual(tool.flight_findings(records), value["expected_findings"])
        # The invariant sweep reports the resealed link count, so the checker
        # rejects the image and the bundle records that rejection.
        actual = json.loads(records["actual.json"])
        self.assertIsNone(actual["failure"])
        self.assertFalse(actual["raw_check"]["clean"])
        self.assertTrue(any("link count" in error for error in actual["raw_check"]["errors"]))
        # A wrong declaration is a reduction signature of its own.
        wrong = dict(value, expected_findings=[dict(value["expected_findings"][0], object=17)])
        records, success = tool.execute(tool.encoded(wrong), BINARY)
        self.assertFalse(success)
        self.assertEqual(json.loads(tool.failure_signature(records))["kind"], "structure")
        empty = dict(value, expected_findings=[])
        records, success = tool.execute(tool.encoded(empty), BINARY)
        self.assertFalse(success)
        # Declaring findings without the category or the ring that retains them
        # is refused before the runner starts.
        for broken in (dict(value, flight_categories=32767 ^ (1 << 12)),
                       dict(value, flight_capacity=1)):
            with self.assertRaisesRegex(ValueError, "expected findings"):
                tool.scenario.validate(tool.encoded(broken))

    def test_v8_fault_commands_are_admission_bounded(self):
        value = lifecycle_scenarios()["window-log"]
        for broken in ({"op": "fault", "class": "read", "index": -1},
                       {"op": "fault", "class": "read", "index": 65536},
                       {"op": "fault", "class": "sync", "index": 0},
                       {"op": "fault", "class": "read"},
                       {"op": "format_fault", "class": "read", "index": 0},
                       {"op": "corrupt", "lba": 4, "offset": 4096, "byte": 0},
                       {"op": "corrupt", "lba": 4, "offset": 0, "byte": 256},
                       {"op": "reseal", "lba": 4, "offset": 31, "byte": 0}):
            with self.assertRaises(ValueError):
                tool.scenario.validate(tool.encoded(
                    dict(value, operations=value["operations"] + [broken])))
        # A format fault belongs to the first operation alone.
        with self.assertRaisesRegex(ValueError, "first operation"):
            tool.scenario.validate(tool.encoded(dict(value, operations=(
                value["operations"] + [{"op": "format_fault", "class": "write", "index": 0}]))))
        # An armed fault no device operation reaches proves nothing.
        never = lifecycle_value([{"op": "fault", "class": "flush", "index": 60000},
                                 {"op": "create", "label": "f", "parent": "root",
                                  "name": "a", "data": "01"}])
        with self.assertRaisesRegex(ValueError, "never tripped"):
            tool.execute(tool.encoded(never), BINARY)
        # The commands belong to version 8 alone.
        legacy = snapshot_fixture()
        for command in ({"op": "fault", "class": "read", "index": 0},
                        {"op": "corrupt", "lba": 4, "offset": 0, "byte": 1},
                        {"op": "reseal", "lba": 4, "offset": 64, "byte": 1}):
            with self.assertRaises(ValueError):
                tool.scenario.validate(tool.encoded(
                    dict(legacy, operations=legacy["operations"] + [command])))

    def test_v8_lifecycle_wire_binds_every_payload_class_to_its_kind(self):
        profile = {"version": 8, "flight_categories": 32767, "flight_sink": None}
        previous = (0,) * 7
        def wire(kind=24, *, tx=0, generation=2, tag=None, enums=(0, 0, 0, 0), region=0,
                 words=(0, 0, 0, 0, 0), present=0, object_id=0, block=0, view=0,
                 span=0, method=0, window=0, group=0, operation=0, parent=0):
            if tag is None:
                tag = tool.payload_tag(kind)
            header = struct.pack("<QQQQQQBI", 0, 0, 1, tx, 0, 0, 0, 1)
            event = (struct.pack("<QQQBB", 1, tx, generation, kind, 0)
                     + struct.pack("<QQQHQI", operation, span, parent, method, window, group)
                     + struct.pack("<BQQQ", present, object_id, block, view)
                     + bytes([tag]) + bytes(enums) + struct.pack("<I", region)
                     + struct.pack("<QQQQQ", *words))
            return header + event
        valid = wire(words=(8, 4, 0, 0, 0))
        self.assertEqual(tool.selected_batch(valid, 0, previous, 1, profile, 0)[0], len(valid))
        # Truncation is refused before any payload is unpacked.
        for length in range(len(valid)):
            with self.assertRaises(ValueError):
                tool.selected_batch(valid[:length], 0, previous, 1, profile, 0)
        admitted = [
            wire(23),
            wire(27, words=(9, 5, 3, 0, 0)),
            wire(35, words=(7, 9, 2, 0, 0)),
            wire(39, generation=0, enums=(0, 1, 0, 0)),
            wire(40, enums=(1, 3, 1, 0), words=(6, 0, 0, 0, 0)),
            wire(42, enums=(0, 6, 1, 1), region=4, group=4),
            wire(43, enums=(0, 6, 0, 0), region=2, group=2),
            wire(45, generation=0, enums=(0, 2, 0, 0)),
            wire(45, enums=(3, 6, 1, 0)),
            wire(46, enums=(1, 0, 0, 0), words=(512, 0, 0, 0, 0)),
            wire(48, enums=(4, 0, 0, 0), words=(512, 33, 0, 0, 0)),
            wire(47, enums=(3, 0, 0, 0), words=(512, 0, 0, 0, 0)),
            wire(50, enums=(2, 0, 0, 0), words=(512, 0, 0, 0, 0)),
            wire(51, enums=(3, 0, 0, 0)),
            wire(52, enums=(3, 16, 0, 0), region=1, words=(0, 0, 40, 0, 0)),
            wire(53, enums=(3, 13, 8, 0), region=2, words=(5, 16, 40, 0, 0)),
            wire(54, enums=(3, 0, 0, 0), words=(11, 0, 0, 0, 0)),
            wire(55, enums=(2, 4, 0, 0), words=(0, 16, 0, 0, 0)),
            wire(56, enums=(1, 0, 0, 0), region=2, words=(16, 0, 8192, 33, 0), span=1,
                 operation=1, method=64),
            wire(59, tx=1),
            wire(61, enums=(4, 0, 0, 0), words=(3, 16, 40, 0, 0)),
            wire(64, enums=(4, 0, 0, 0), words=(0, 0, 24, 0, 0)),
        ]
        for value in admitted:
            tool.selected_batch(value, 0, previous, 1, profile, 0)
        # Every extended kind the core can emit is admitted at its own code.
        def canonical(kind):
            if 23 <= kind <= 26:
                return dict(words=(8, 4, 0, 0, 0))
            if 27 <= kind <= 31:
                return dict(words=(9, 5, 3, 0, 0))
            if 32 <= kind <= 38:
                return dict(words=(7, 9, 2, 0, 0))
            if kind == 39:
                return dict(generation=0, enums=(0, 1, 0, 0))
            if kind == 40:
                return dict(enums=(1, 3, 1, 0), words=(6, 0, 0, 0, 0))
            if 41 <= kind <= 44:
                return dict(enums=(0, 6, 0, 0), region=2, group=2)
            if kind == 45:
                return dict(enums=(2, 4, 1, 0), words=(6, 0, 0, 0, 0))
            if 46 <= kind <= 50:
                stage = {46: 1, 47: 3, 48: 4, 49: 5, 50: 2}[kind]
                block = 33 if kind in (48, 49) else 0
                return dict(enums=(stage, 0, 0, 0), words=(512, block, 0, 0, 0))
            if kind in (51, 54):
                ordinal = 11 if kind == 54 else 0
                return dict(enums=(3, 0, 0, 0), words=(ordinal, 0, 0, 0, 0))
            if kind == 52:
                return dict(enums=(3, 13, 0, 0), region=1, words=(0, 0, 40, 0, 0))
            if kind == 53:
                return dict(enums=(3, 13, 8, 0), region=2, words=(5, 16, 40, 0, 0))
            if kind == 55:
                return dict(enums=(2, 4, 0, 0), words=(0, 16, 0, 0, 0))
            if 56 <= kind <= 58:
                return dict(enums=(1, 0, 0, 0), region=2, words=(16, 0, 8192, 33, 0),
                            span=1, operation=1, method=64)
            if kind in (59, 60):
                return dict(tx=1)
            return dict(enums=(1, 0, 0, 0), words=(3, 16, 40, 0, 0))
        for kind in range(23, 65):
            tool.selected_batch(wire(kind, **canonical(kind)), 0, previous, 1, profile, 0)
            # An empty mask refuses the same record, whatever its kind.
            with self.assertRaises(ValueError):
                tool.selected_batch(wire(kind, **canonical(kind)), 0, previous, 1,
                                    dict(profile, flight_categories=0), 0)
        for kind in (0, 65, 200, 255):
            with self.assertRaises(ValueError):
                tool.selected_batch(wire(kind), 0, previous, 1, profile, 0)
        refused = [
            # Tag, presence and reserved-byte consistency.
            wire(24, tag=0), wire(24, tag=2), wire(1, tag=1), wire(59, tx=1, tag=4),
            wire(24, enums=(1, 0, 0, 0)), wire(24, region=1), wire(24, words=(0, 0, 1, 0, 0)),
            wire(27, words=(0, 0, 0, 1, 0)), wire(35, words=(0, 0, 0, 0, 1)),
            # Mount stage, slot, damaged tail, count and generation.
            wire(39, generation=0, enums=(0, 0, 0, 0)), wire(39, generation=0, enums=(0, 7, 0, 0)),
            wire(39, generation=0, enums=(4, 1, 0, 0)), wire(39, generation=0, enums=(0, 1, 2, 0)),
            wire(39, generation=0, enums=(0, 1, 0, 2)), wire(39, enums=(0, 1, 0, 0)),
            wire(39, generation=0, enums=(0, 3, 0, 0)),
            wire(40, enums=(0, 6, 0, 0)), wire(41, enums=(0, 3, 0, 0)),
            wire(43, enums=(0, 6, 0, 1), region=2, group=2),
            wire(40, enums=(0, 3, 0, 0), region=3),
            wire(45, generation=0, enums=(0, 2, 1, 0)),
            wire(45, generation=0, enums=(0, 2, 0, 0), words=(6, 0, 0, 0, 0)),
            wire(44, generation=0, enums=(0, 6, 0, 0)),
            wire(40, enums=(0, 3, 0, 0), words=(0, 1, 0, 0, 0)),
            # Format stage, device size and publication address.
            wire(46, enums=(0, 0, 0, 0), words=(512, 0, 0, 0, 0)),
            wire(46, enums=(6, 0, 0, 0), words=(512, 0, 0, 0, 0)),
            wire(46, enums=(2, 0, 0, 0), words=(512, 0, 0, 0, 0)),
            wire(49, enums=(4, 0, 0, 0), words=(512, 33, 0, 0, 0)),
            wire(46, enums=(1, 0, 0, 0), words=(0, 0, 0, 0, 0)),
            wire(46, enums=(1, 0, 0, 0), words=(512, 33, 0, 0, 0)),
            wire(48, enums=(4, 0, 0, 0), words=(512, 0, 0, 0, 0)),
            wire(46, enums=(1, 1, 0, 0), words=(512, 0, 0, 0, 0)),
            wire(46, enums=(1, 0, 0, 0), region=1, words=(512, 0, 0, 0, 0)),
            # Verify scope, phase, finding class, ordinal and location.
            wire(51, enums=(0, 0, 0, 0)), wire(51, enums=(4, 0, 0, 0)),
            wire(51, enums=(3, 1, 0, 0)), wire(52, enums=(3, 0, 0, 0)),
            wire(52, enums=(3, 17, 0, 0)), wire(53, enums=(3, 13, 12, 0)),
            wire(52, enums=(3, 13, 1, 0)), wire(53, enums=(3, 13, 0, 0)),
            wire(52, enums=(3, 13, 0, 0), words=(1, 0, 0, 0, 0)),
            wire(51, enums=(3, 0, 0, 0), region=1),
            wire(54, enums=(3, 0, 0, 0), words=(0, 16, 0, 0, 0)),
            wire(51, enums=(3, 0, 0, 1)), wire(51, enums=(3, 0, 0, 0), words=(0, 0, 0, 1, 0)),
            # Staged data and view descent domains.
            wire(56, enums=(0, 0, 0, 0)), wire(56, enums=(4, 0, 0, 0)),
            wire(56, enums=(1, 1, 0, 0)), wire(56, words=(0, 0, 0, 0, 1)),
            wire(61, enums=(0, 0, 0, 0), words=(0, 16, 0, 0, 0)),
            wire(61, enums=(5, 0, 0, 0), words=(0, 16, 0, 0, 0)),
            wire(61, enums=(1, 0, 0, 0), words=(0, 0, 0, 0, 0)),
            wire(61, enums=(1, 1, 0, 0), words=(0, 16, 0, 0, 0)),
            wire(61, enums=(1, 0, 0, 0), region=1, words=(0, 16, 0, 0, 0)),
            # Identity rules the extended kinds inherit.
            wire(24, tx=1), wire(59), wire(65), wire(24, generation=0),
            wire(24, group=2), wire(61, enums=(1, 0, 0, 0), words=(0, 16, 0, 0, 0), group=1),
        ]
        for value in refused:
            with self.assertRaises(ValueError):
                tool.selected_batch(value, 0, previous, 1, profile, 0)

    def test_v8_legacy_profiles_and_magics_stay_separate(self):
        # Versions 1 to 7 and version 9 refuse every extended kind and the
        # version-8 artifact magic; version 8 refuses the version-6 magic.
        for version, mask in ((6, 127), (9, 127)):
            profile = {"version": version, "flight_categories": mask, "flight_sink": None}
            header = struct.pack("<QQQQQQBI", 0, 0, 1, 0, 0, 0, 0, 1)
            for kind in (23, 39, 46, 51, 56, 59, 61):
                event = struct.pack("<QQQBBQQQHQIBQQQ", 1, 0, 2, kind, 0,
                                    0, 1, 0, 1, 0, 0, 0, 0, 0, 0)
                with self.assertRaises(ValueError):
                    tool.selected_batch(header + event, 0, (0,) * 7, 1, profile, 0)
        records, success = tool.execute(tool.encoded(lifecycle_fixture()), BINARY)
        self.assertTrue(success)
        self.assertEqual(records["flight-recorder.bin"][:8], b"AFSFLT06")
        for magic in (b"AFSFLT05", b"AFSFLT07"):
            edited = magic + records["flight-recorder.bin"][8:]
            with self.assertRaisesRegex(ValueError, "flight version"):
                tool.validate_trace(dict(records, **{"flight-recorder.bin": edited}))
        legacy, _ = tool.execute(tool.encoded(snapshot_fixture()), BINARY)
        self.assertEqual(legacy["flight-recorder.bin"][:8], b"AFSFLT05")

    def test_v8_profiles_preserve_images_and_replay_in_a_fresh_process(self):
        for pages in (2, 4, 8, "unlimited"):
            plain, success = tool.execute(tool.encoded(lifecycle_fixture(pages, 0, 1)), BINARY)
            self.assertTrue(success)
            for mask in (0, 1 << 10, 1 << 12, 1 << 14, 32767):
                for capacity in (1, 256):
                    value = lifecycle_fixture(pages, mask, capacity,
                                              {"capacity": 1, "disconnect_before": 3})
                    records, success = tool.execute(tool.encoded(value), BINARY)
                    self.assertTrue(success, (pages, mask, capacity))
                    self.assertEqual(records["flight-recorder.bin"][:8], b"AFSFLT06")
                    for role in ("start.img", "result.img", "block-io.afstrace", "actual.json"):
                        self.assertEqual(records[role], plain[role], (pages, mask, capacity))
            with tempfile.TemporaryDirectory(prefix="afsplus-lifecycle-replay-") as temporary:
                path = Path(temporary) / "bundle"
                tool.bundle.publish(path, records)
                self.assertTrue(tool.replay(path, BINARY))
                result = subprocess.run(
                    [sys.executable, str(tool.ROOT / "tools/afsptest.py"), "--runner", str(BINARY),
                     "replay", str(path)], capture_output=True, text=True, timeout=600)
                self.assertEqual(result.returncode, 0, result.stderr)

    def test_v8_changed_event_byte_fails_exact_replay(self):
        records, success = tool.execute(tool.encoded(lifecycle_fixture()), BINARY)
        self.assertTrue(success)
        wire = records["flight-recorder.bin"]
        # First event of the explicit pre-mount batch: FormatBegin with its
        # validation stage and the device block count.
        first = 28 + 53
        self.assertEqual(wire[first + 24], 46)
        self.assertEqual(wire[first + 89], 5)
        self.assertEqual(wire[first + 90], 1)
        self.assertEqual(struct.unpack_from("<Q", wire, first + 98)[0], 512)
        for offset, fmt, number in ((first + 89, "B", 0), (first + 90, "B", 2),
                                    (first + 98, "Q", 0), (first + 106, "Q", 1),
                                    (first + 94, "I", 1), (first + 24, "B", 65),
                                    (first + 90, "B", 2)):
            edited = bytearray(wire)
            struct.pack_into("<" + fmt, edited, offset, number)
            self.assertNotEqual(bytes(edited), wire)
            with self.assertRaises(ValueError):
                tool.validate_trace(dict(records, **{"flight-recorder.bin": bytes(edited)}))
        with tempfile.TemporaryDirectory(prefix="afsplus-lifecycle-edit-") as temporary:
            path = Path(temporary) / "bundle"
            edited = bytearray(wire)
            # A value inside its own domain still differs from the recorded run.
            struct.pack_into("<Q", edited, first + 98, 511)
            tool.bundle.publish(path, dict(records, **{"flight-recorder.bin": bytes(edited)}))
            with self.assertRaisesRegex(ValueError, "semantic replay mismatch"):
                tool.replay(path, BINARY)

    def test_v8_minimization_and_cuts_keep_the_lifecycle_policy(self):
        value = lifecycle_fixture(2, 32767, 1, {"capacity": 1, "disconnect_before": 2})
        value["operations"][:0] = [
            {"op": "create", "label": "spare", "parent": "root", "name": "temp", "data": ""},
            {"op": "unlink", "label": "spare"}]
        value["expected"][1]["data"] = "ffff"
        records, success = tool.execute(tool.encoded(value), BINARY)
        self.assertFalse(success)
        signature = json.loads(tool.failure_signature(records))
        self.assertEqual(signature["flight_categories"], 32767)
        self.assertEqual(signature["flight_sink"], value["flight_sink"])
        self.assertEqual(signature["snapshot_limits"], value["snapshot_limits"])
        with tempfile.TemporaryDirectory(prefix="afsplus-lifecycle-minimize-") as temporary:
            root = Path(temporary)
            tool.bundle.publish(root / "original", records)
            result = tool.minimize(root / "original", root / "reduced", BINARY, max_runs=32)
            self.assertLessEqual(result["operations"], len(value["operations"]))
            reduced = tool.bundle.read_bundle(root / "reduced")
            self.assertEqual(tool.failure_signature(reduced), tool.failure_signature(records))
            self.assertFalse(tool.replay(root / "reduced", BINARY))
        cut = lifecycle_fixture(2, 32767, 256)
        cut["operations"] = [{"op": "create", "label": "f", "parent": "root", "name": "a", "data": "01"}]
        cut["expected"] = [{"path": ["a"], "kind": "file", "data": "01"}]
        for variant in range(5):
            records, _ = tool.execute(tool.encoded(cut), BINARY,
                fault={"version": 1, "kind": "power-cut-v1", "operation": 0, "offset": 1,
                       "variant": variant})
            tool.validate_trace(records)
            self.assertEqual(records["flight-recorder.bin"][:8], b"AFSFLT06")

    def test_v9_linked_observation_decodes_aliases_targets_and_protection(self):
        lines = ["file 616c696173 2 7 0 0 4142 0:4096:0", "file 636c6f6e65 1 7 1 1 42 -",
                 "directory 6d6f766564 1 0", "file 6d6f766564,66696c65 2 7 0 0 4142 0:4096:0",
                 "symlink 6d6f766564,6c696e6b 1 0 2e2e"]
        entries = []
        for line in lines:
            entries.append(tool.linked_entry(line.split(" "), entries))
        self.assertEqual((entries[3]["alias"], entries[4]["target"]), (0, ".."))
        for index, bad in ((3, "file 6d6f766564,66696c65 2 6 0 0 4142 0:4096:0"),
                           (3, "file 6d6f766564,66696c65 2 7 1 0 4142 0:4096:0"),
                           (3, "file 6d6f766564,66696c65 2 7 0 1 4142 0:4096:0"),
                           (3, "file 6d6f766564,66696c65 2 7 0 0 4142 -"),
                           (1, "file 636c6f6e65 0 7 1 1 42 -"), (1, "file 636c6f6e65 1 7 2 1 42 -"),
                           (1, "file 636c6f6e65 1 7 1 2 42 -"),
                           (1, "file 636c6f6e65 1 7 1 1 42 0:4096:0,4096:4096:0"),
                           (1, "file 636c6f6e65 1 7 1 1 42 4096:4096:0,0:4096:1"),
                           (4, "symlink 6d6f766564,6c696e6b 1 0 -"), (2, "directory 6d6f766564 1 01"),
                           (1, "file 616c696173 1 7 1 0 42 0:4096:0"),
                           (4, "symlink 6d6f766564,6c696e6b 1 0"),
                           (0, "fifo 616c696173 1 0")):
            with self.assertRaises(ValueError, msg=bad):
                tool.linked_entry(bad.split(" "), list(entries[:index]))

    def test_v9_linked_bundles_replay_and_wrong_link_values_fail(self):
        for pages in (2, 4, 8, "unlimited"):
            value = linked_fixture(pages)
            records, success = tool.execute(tool.encoded(value), BINARY)
            actual = json.loads(records["actual.json"])
            self.assertTrue(success, actual["entries"])
            self.assertEqual((actual["version"], actual["cache_pages"]), (5, pages))
            self.assertTrue(records["flight-recorder.bin"].startswith(b"AFSFLT05"))
            with self.assertRaises(ValueError):
                tool.bind_cache_profile(value, dict(actual, version=4))
            with tempfile.TemporaryDirectory(prefix="afsplus-linked-replay-") as temporary:
                path = Path(temporary) / "bundle"
                tool.bundle.publish(path, records)
                self.assertTrue(tool.replay(path, BINARY))
        for index, field, wrong in ((0, "links", 1), (1, "protection", 6), (4, "target", "../other"),
                                    (1, "data", "43" + "00" * 4094 + "a1ff"), (3, "alias", 1)):
            value = linked_fixture()
            value["expected"][index][field] = wrong
            records, success = tool.execute(tool.encoded(value), BINARY)
            self.assertFalse(success, field)
            self.assertEqual(tool.admit_run(records)[0]["outcome"], "failure")
            self.assertEqual(json.loads(tool.failure_signature(records))["kind"], "state")
            self.assertFalse(tool.verify_replay(records, BINARY, tool.bundle.DEFAULT_FILE_BYTES,
                                               tool.bundle.DEFAULT_TOTAL_BYTES))

    def test_captured_observation_distinguishes_empty_error_and_history(self):
        root = "1 directory 0 0 1 0 0 0 0 0 0 0 0"
        file = "2 file 1 4096 1 0 0 0 0 0 0 0 1"
        lines = ["snapshots ok 1", "snapshot 1 2 2 1", "root " + root,
                 "entry 66696c65 " + file + " 61 1", "range 0 4096 0"]
        actual = tool.captured_observation(lines)
        self.assertIsNone(actual["snapshot_inspection_error"])
        self.assertEqual(actual["snapshots"][0]["entries"][0]["data"], "61")
        self.assertEqual(tool.captured_observation(["snapshots ok 0"])["snapshots"], [])
        error = tool.captured_observation(["snapshots error 626164"])
        self.assertIsNone(error["snapshots"])
        self.assertEqual(error["snapshot_inspection_error"], "bad")
        for cut in range(len(lines)):
            with self.assertRaises(ValueError):
                tool.captured_observation(lines[:cut])
        for bad in (lines + ["extra"], ["snapshots ok 17"],
                    ["snapshots error 626164", "extra"],
                    lines[:-1] + ["range 0 4096 2"],
                    lines[:3] + ["entry 66696c65 " + file + " - 0"],
                    lines[:3] + ["entry 2f " + file + " 61 1", lines[-1]]):
            with self.assertRaises(ValueError):
                tool.captured_observation(bad)

    def test_captured_runner_exports_history_after_live_mutation(self):
        for pages in (2, 4, 8, "unlimited"):
            commands = (f"AFSPSC07\nformat 4096 512 64 8 {pages} 256 127 0 none 4096 16 8\n"
                        "create f root 66696c65 61\nsnapshot_create first\n"
                        "write f 0 62\nsnapshot_create second\n"
                        "write f 0 63\nremount\n").encode()
            run = subprocess.run([str(BINARY)], input=commands, capture_output=True, check=True)
            records = tool.unframe(io.BytesIO(run.stdout), tool.bundle.DEFAULT_FILE_BYTES, tool.bundle.DEFAULT_TOTAL_BYTES)
            actual = tool.observation(records["actual.wire"])
            self.assertEqual(actual["version"], 4)
            self.assertEqual(actual["cache_pages"], pages)
            self.assertIsNone(actual["failure"])
            self.assertTrue(tool.structural_success(actual))
            self.assertEqual(actual["entries"], [{"path": ["file"], "kind": "file", "data": "63"}])
            self.assertEqual([view["id"] for view in actual["snapshots"]], [1, 2])
            self.assertEqual([view["entries"][0]["data"] for view in actual["snapshots"]], ["61", "62"])

    def test_v7_bundles_bind_full_expected_history_and_replay(self):
        for pages in (2, 4, 8, "unlimited"):
            value = snapshot_fixture(pages)
            records, success = tool.execute(tool.encoded(value), BINARY)
            self.assertTrue(success)
            self.assertEqual(json.loads(records["expected.json"]),
                             {"entries": value["expected"], "snapshots": value["expected_snapshots"]})
            self.assertEqual(tool.admit_run(records)[0]["outcome"], "pass")
            self.assertTrue(tool.verify_replay(records, BINARY, tool.bundle.DEFAULT_FILE_BYTES,
                                              tool.bundle.DEFAULT_TOTAL_BYTES))
        for change in ("contents", "metadata", "allocation", "registry"):
            value = snapshot_fixture()
            view = value["expected_snapshots"][0]
            if change == "contents": view["entries"][0]["data"] = "63"
            elif change == "metadata": view["entries"][0]["metadata"]["protection"] = 1
            elif change == "allocation": view["entries"][0]["allocation"][0]["unwritten"] = True
            else: value["expected_snapshots"] = []
            records, success = tool.execute(tool.encoded(value), BINARY)
            self.assertFalse(success, change)
            self.assertEqual(tool.admit_run(records)[0]["outcome"], "failure")
            self.assertEqual(json.loads(tool.failure_signature(records))["kind"], "snapshot-state")
            self.assertFalse(tool.verify_replay(records, BINARY, tool.bundle.DEFAULT_FILE_BYTES,
                                               tool.bundle.DEFAULT_TOTAL_BYTES))

    def test_v7_snapshot_handle_lifecycle_and_deletion(self):
        for action in ("busy", "stale", "closed", "reopen_deleted"):
            value = snapshot_fixture()
            value["operations"] = value["operations"][:3]
            value["expected"][0]["data"] = "61"
            if action == "busy":
                value["operations"].append({"op": "snapshot_delete", "label": "s"})
            elif action == "stale":
                value["operations"] += [{"op": "remount"}, {"op": "snapshot_inspect", "label": "s"}]
            else:
                value["operations"] += [{"op": "snapshot_close", "label": "s"},
                                          {"op": "snapshot_delete", "label": "s"}]
                value["expected_snapshots"] = []
                if action == "reopen_deleted":
                    value["operations"].append({"op": "snapshot_open", "label": "s"})
            records, success = tool.execute(tool.encoded(value), BINARY)
            self.assertEqual(success, action == "closed")
            actual = json.loads(records["actual.json"])
            self.assertEqual(actual["snapshots"], value["expected_snapshots"])
            if action == "stale":
                self.assertEqual(actual["failure"]["error"], "snapshot handle not open")
            elif action != "closed":
                self.assertIsNotNone(actual["failure"])
            self.assertEqual(tool.verify_replay(records, BINARY, tool.bundle.DEFAULT_FILE_BYTES,
                                                tool.bundle.DEFAULT_TOTAL_BYTES), success)

    def test_v7_snapshot_publication_boundaries_replay_exact_registry(self):
        for pages in (2, 4, 8, "unlimited"):
            for deleting in (False, True):
                value = snapshot_fixture(pages)
                value["operations"] = value["operations"][:2]
                value["expected"][0]["data"] = "61"
                historical = value["expected_snapshots"]
                if deleting:
                    value["operations"].append({"op": "snapshot_delete", "label": "s"})
                    value["expected_snapshots"] = []
                records, success = tool.execute(tool.encoded(value), BINARY)
                self.assertTrue(success)
                wire, offset, previous = records["flight-recorder.bin"], 28, (0,) * 7
                bounds = []
                for index in range(len(value["operations"])):
                    _, first, last, _, _ = struct.unpack("<IQQQB", wire[offset:offset + 29])
                    bounds.append((first, last))
                    offset, previous = tool.selected_batch(wire, offset + 29, previous, 256, value, index)
                target = len(bounds) - 1
                for after in (False, True):
                    candidate = dict(value, expected_snapshots=([] if after == deleting else historical))
                    first, last = bounds[target]
                    fault = {"version": 1, "kind": "power-cut-v1", "operation": target,
                             "offset": last - first if after else 0, "variant": 0}
                    cut, passed = tool.execute(tool.encoded(candidate), BINARY, fault=fault)
                    self.assertTrue(passed, (pages, deleting, after))
                    self.assertTrue(tool.verify_replay(cut, BINARY, tool.bundle.DEFAULT_FILE_BYTES,
                                                       tool.bundle.DEFAULT_TOTAL_BYTES))

    def test_v7_large_observed_sparse_snapshot_records_a_semantic_failure(self):
        value = snapshot_fixture()
        value["operations"] = [value["operations"][0],
            {"op": "truncate", "label": "f", "size": 2 * 1024 * 1024},
            {"op": "snapshot_create", "label": "s"}, {"op": "unlink", "label": "f"}]
        value["expected"], value["expected_snapshots"] = [], []
        records, success = tool.execute(tool.encoded(value), BINARY)
        self.assertFalse(success)
        actual = json.loads(records["actual.json"])
        self.assertIsNone(actual["snapshot_inspection_error"])
        self.assertEqual(actual["entries"], [])
        entry = actual["snapshots"][0]["entries"][0]
        self.assertEqual(entry["data"], "61" + "00" * (2 * 1024 * 1024 - 1))
        self.assertEqual(entry["metadata"]["size"], 2 * 1024 * 1024)
        self.assertEqual(tool.admit_run(records)[0]["outcome"], "failure")
        self.assertEqual(json.loads(tool.failure_signature(records))["kind"], "snapshot-state")
        self.assertFalse(tool.verify_replay(records, BINARY, tool.bundle.DEFAULT_FILE_BYTES,
                                           tool.bundle.DEFAULT_TOTAL_BYTES))
        with self.assertRaises(ValueError):
            tool.scenario.captured_views(actual["snapshots"])
        tool.scenario.captured_views(actual["snapshots"], observed=True)

    def test_v6_object_wire_rejects_truncation_and_inconsistent_presence(self):
        profile = {"version": 6, "flight_categories": 127, "flight_sink": None}
        previous = (0,) * 7
        def wire(kind=20, present=1, object_id=9, block=0, view=3):
            tx = int(kind == 1)
            header = struct.pack("<QQQQQQBI", 0, 0, 1, tx, 0, 0, 0, 1)
            event = struct.pack("<QQQBBQQQHQIBQQQ", 1, tx, 2, kind, 0,
                                0, 0, 0, 0, 0, 0, present, object_id, block, view)
            return header + event
        valid = wire()
        self.assertEqual(tool.selected_batch(valid, 0, previous, 1, profile, 0)[0], len(valid))
        for length in range(len(valid)):
            with self.assertRaises(ValueError):
                tool.selected_batch(valid[:length], 0, previous, 1, profile, 0)
        for bad in (wire(present=0), wire(present=2), wire(block=8),
                    wire(kind=22, block=8), wire(kind=23),
                    wire(kind=1), wire(kind=1, present=0),
                    wire(kind=1, present=0, object_id=0, view=0, block=8)):
            with self.assertRaises(ValueError):
                tool.selected_batch(bad, 0, previous, 1, profile, 0)
        # Resolved addresses are observations, including an invalid address;
        # later metadata checks, not this decoder, determine filesystem validity.
        for valid in (wire(kind=21, block=0xffffffffffffffff), wire(kind=22),
                      wire(kind=1, present=0, object_id=0, view=0)):
            tool.selected_batch(valid, 0, previous, 1, profile, 0)

    def test_v6_object_profiles_preserve_images_and_replay(self):
        for pages in (2, 4, 8, "unlimited"):
            value = fixture()
            value.update(version=5, flight_capacity=256, flight_categories=63, flight_sink=None)
            value["volume"]["tree_cache_pages"] = pages
            plain, success = tool.execute(tool.encoded(value), BINARY)
            self.assertTrue(success)
            for mask in (0, 64, 127):
                for capacity in (1, 256):
                    observed = dict(value, version=6, flight_categories=mask,
                                    flight_capacity=capacity,
                                    flight_sink={"capacity": 1, "disconnect_before": 2})
                    records, success = tool.execute(tool.encoded(observed), BINARY)
                    self.assertTrue(success)
                    self.assertEqual(records["flight-recorder.bin"][:8], b"AFSFLT05")
                    for role in ("start.img", "result.img", "block-io.afstrace", "actual.json"):
                        self.assertEqual(records[role], plain[role])
                    with tempfile.TemporaryDirectory(prefix="afsplus-object-replay-") as temporary:
                        path = Path(temporary) / "bundle"
                        tool.bundle.publish(path, records)
                        self.assertTrue(tool.replay(path, BINARY))
                    if mask == 64 and capacity == 256:
                        wire = records["flight-recorder.bin"]
                        offset = 28
                        kinds = set()
                        for _ in range(struct.unpack_from("<I", wire, 8)[0]):
                            offset += 29
                            batch = struct.unpack_from("<QQQQQQBI", wire, offset)
                            offset += 53
                            for _ in range(batch[-1]):
                                kind = wire[offset + 24]
                                kinds.add(kind)
                                self.assertEqual(wire[offset + 64], 1)
                                self.assertEqual(struct.unpack_from("<Q", wire, offset + 81)[0], 0)
                                bad = bytearray(wire)
                                bad[offset + 64] = 0
                                with self.assertRaises(ValueError):
                                    tool.validate_trace(dict(records, **{"flight-recorder.bin": bytes(bad)}))
                                if kind in (20, 22):
                                    bad = bytearray(wire)
                                    struct.pack_into("<Q", bad, offset + 73, 1)
                                    with self.assertRaises(ValueError):
                                        tool.validate_trace(dict(records, **{"flight-recorder.bin": bytes(bad)}))
                                offset += 89
                        self.assertTrue({20, 21} <= kinds)
                        self.assertEqual(offset, len(wire))

    def test_v5_deferred_groups_join_calls_and_survive_replay(self):
        self.check_deferred_groups(5, 63)

    def test_v6_deferred_groups_join_calls_and_survive_replay(self):
        self.check_deferred_groups(6, 127)

    def check_deferred_groups(self, version, mask):
        for pages in (2, 4, 8, "unlimited"):
            value = fixture()
            value.update(version=version, flight_capacity=256, flight_categories=mask, flight_sink=None)
            value["volume"]["tree_cache_pages"] = pages
            value["operations"] = [value["operations"][0],
                {"op": "window_write", "label": "f", "offset": 1, "data": "42"},
                {"op": "window_fsync"},
                {"op": "window_truncate", "label": "f", "size": 1},
                {"op": "window_fsync"}, {"op": "window_commit"}, {"op": "remount"}]
            value["expected"][0]["data"] = "00"
            records, success = tool.execute(tool.encoded(value), BINARY)
            self.assertTrue(success)
            wire = records["flight-recorder.bin"]
            offset, events = 28, []
            for _ in range(struct.unpack_from("<I", wire, 8)[0]):
                offset += 29
                batch = struct.unpack_from("<QQQQQQBI", wire, offset)
                offset += 53
                self.assertEqual(batch[0], 0)
                for _ in range(batch[-1]):
                    events.append(struct.unpack_from("<QQQBBQQQHQI", wire, offset))
                    offset += 89 if version == 6 else 64
            durable = [event for event in events if event[3] == 15]
            self.assertEqual([event[-1] for event in durable], [1, 2])
            self.assertEqual({event[-2] for event in durable}, {1})
            roots = {event[5] for event in events if event[-2] == 1 and event[5]}
            self.assertGreaterEqual(len(roots), 5)
            with tempfile.TemporaryDirectory(prefix="afsplus-window-replay-") as temporary:
                path = Path(temporary) / "bundle"
                tool.bundle.publish(path, records)
                self.assertTrue(tool.replay(path, BINARY))
            for cut_offset, expected_data in ((0, "00ff"), (3, "0042")):
                cut_value = json.loads(json.dumps(value))
                cut_value["expected"][0]["data"] = expected_data
                cut_records, cut_success = tool.execute(tool.encoded(cut_value), BINARY,
                    fault={"version": 1, "kind": "power-cut-v1", "operation": 2,
                           "offset": cut_offset, "variant": 0})
                self.assertTrue(cut_success)
                tool.validate_trace(cut_records)
                with tempfile.TemporaryDirectory(prefix="afsplus-window-cut-") as temporary:
                    path = Path(temporary) / "bundle"
                    tool.bundle.publish(path, cut_records)
                    self.assertTrue(tool.replay(path, BINARY))

    def test_v5_api_window_profiles_preserve_images_and_replay(self):
        for pages in (2, 4, 8, "unlimited"):
            baseline = fixture()
            baseline.update(version=4, flight_capacity=256, flight_categories=15, flight_sink=None)
            baseline["volume"]["tree_cache_pages"] = pages
            plain, success = tool.execute(tool.encoded(baseline), BINARY)
            self.assertTrue(success)
            for mask in (0, 2, 16, 32, 48, 63):
                for capacity in (1, 256):
                    value = dict(baseline, version=5, flight_capacity=capacity,
                                 flight_categories=mask,
                                 flight_sink={"capacity": 2, "disconnect_before": 2})
                    records, success = tool.execute(tool.encoded(value), BINARY)
                    self.assertTrue(success)
                    self.assertEqual(records["flight-recorder.bin"][:8], b"AFSFLT04")
                    for role in ("start.img", "result.img", "block-io.afstrace", "actual.json"):
                        self.assertEqual(records[role], plain[role])
                    with tempfile.TemporaryDirectory(prefix="afsplus-api-flight-") as temporary:
                        path = Path(temporary) / "bundle"
                        tool.bundle.publish(path, records)
                        self.assertTrue(tool.replay(path, BINARY))

    def test_v5_rejects_corrupted_api_context(self):
        value = fixture()
        value.update(version=5, flight_capacity=256, flight_categories=63, flight_sink=None)
        value["volume"]["tree_cache_pages"] = 2
        records, success = tool.execute(tool.encoded(value), BINARY)
        self.assertTrue(success)
        # Header, first operation, batch header, then the first 64-byte event.
        start = 28 + 29 + 53
        wire = records["flight-recorder.bin"]
        self.assertEqual(wire[start + 24], 8)  # ApiBegin
        for relative, fmt, replacement in ((26, "Q", 0), (34, "Q", 0),
                                             (42, "Q", 999), (50, "H", 65535),
                                             (52, "Q", 999), (60, "I", 1)):
            damaged = bytearray(wire)
            struct.pack_into("<" + fmt, damaged, start + relative, replacement)
            bad = dict(records, **{"flight-recorder.bin": bytes(damaged)})
            with self.assertRaises(ValueError):
                tool.validate_trace(bad)

    def test_v4_filtered_and_live_profiles_replay_without_changing_images(self):
        for pages in (2, 4, 8, "unlimited"):
            baseline = fixture()
            baseline.update(version=3, flight_capacity=1)
            baseline["volume"]["tree_cache_pages"] = pages
            plain, success = tool.execute(tool.encoded(baseline), BINARY)
            self.assertTrue(success)
            for mask in (0, 1, 2, 4, 8, 15):
                for sink in (None, {"capacity": 1, "disconnect_before": 2}):
                    value = dict(baseline, version=4, flight_categories=mask, flight_sink=sink)
                    records, success = tool.execute(tool.encoded(value), BINARY)
                    self.assertTrue(success)
                    self.assertEqual(records["flight-recorder.bin"][:8], b"AFSFLT03")
                    for role in ("start.img", "result.img", "block-io.afstrace", "actual.json"):
                        self.assertEqual(records[role], plain[role])
                    with tempfile.TemporaryDirectory(prefix="afsplus-selected-flight-") as temporary:
                        path = Path(temporary) / "bundle"
                        tool.bundle.publish(path, records)
                        before = {p.name: p.read_bytes() for p in path.iterdir()}
                        replay = self.cli("replay", path)
                        self.assertEqual(replay.returncode, 0, replay.stderr.decode())
                        self.assertEqual(before, {p.name: p.read_bytes() for p in path.iterdir()})

    def test_v4_runtime_failure_does_not_invent_commit_or_disconnect_events(self):
        for pages in (2, 4, 8, "unlimited"):
            for mask in (8, 15):
                value = fixture()
                first = value["operations"][0]
                value["operations"] = [first, dict(first, label="other", data="99"), {"op": "sync"}]
                value["expected"][0]["data"] = "00ff"
                value.update(version=4, flight_capacity=1, flight_categories=mask,
                             flight_sink={"capacity": 1, "disconnect_before": 1})
                value["volume"]["tree_cache_pages"] = pages
                records, success = tool.execute(tool.encoded(value), BINARY)
                self.assertFalse(success)
                actual = json.loads(records["actual.json"])
                self.assertEqual(actual["failure"]["operation"], 1)
                self.assertEqual(actual["entries"], value["expected"])
                flight = records["flight-recorder.bin"]
                self.assertEqual(struct.unpack_from("<I", flight, 8)[0], 2)
                first_batch = struct.unpack_from("<QQQQQQBI", flight, 28+29)
                second = 28+29+53+26*first_batch[-1]
                self.assertEqual(flight[second+28], 0)
                last_batch = struct.unpack_from("<QQQQQQBI", flight, second+29)
                self.assertEqual(last_batch[:-1], first_batch[:-1])
                self.assertEqual(last_batch[-1], 0)
                self.assertEqual(last_batch[-2], 0)
                tool.validate_trace(records)

    def test_v4_selected_counters_and_profile_are_independently_checked(self):
        value = fixture()
        value.update(version=4, flight_capacity=1, flight_categories=2,
                     flight_sink={"capacity": 1, "disconnect_before": 2})
        value["volume"]["tree_cache_pages"] = 2
        records, success = tool.execute(tool.encoded(value), BINARY)
        self.assertTrue(success)
        batch, event = 28 + 29, 28 + 29 + 53
        self.assertEqual(struct.unpack_from("<QQQQQQBI", records["flight-recorder.bin"], batch),
                         (2, 3, 6, 1, 1, 2, 0, 1))
        changes = [(16, "I", 15), (20, "I", 0), (24, "I", 3),
            (batch, "Q", 0), (batch+8, "Q", 0), (batch+16, "Q", 7),
            (batch+24, "Q", 0), (batch+32, "Q", 2), (batch+40, "Q", 0),
            (batch+48, "B", 1), (batch+49, "I", 2),
            (event, "Q", 2), (event, "Q", 7), (event+8, "Q", 0),
            (event+16, "Q", 0), (event+24, "B", 1), (event+25, "B", 2)]
        for offset, fmt, number in changes:
            flight = bytearray(records["flight-recorder.bin"])
            struct.pack_into("<"+fmt, flight, offset, number)
            with self.assertRaisesRegex(ValueError, "flight"):
                tool.validate_trace(dict(records, **{"flight-recorder.bin": bytes(flight)}))
        for wire in (records["flight-recorder.bin"][:-1], records["flight-recorder.bin"]+b"x",
                     b"AFSFLT02"+records["flight-recorder.bin"][8:]):
            with self.assertRaisesRegex(ValueError, "flight"):
                tool.validate_trace(dict(records, **{"flight-recorder.bin": wire}))
        for mask, sink in [(16, None), (True, None), (0, {"capacity": 0, "disconnect_before": None}),
                           (0, {"capacity": 1, "disconnect_before": -1}),
                           (0, {"capacity": 1, "disconnect_before": 1025})]:
            with self.assertRaises(ValueError):
                tool.scenario.validate(tool.encoded(dict(value, flight_categories=mask, flight_sink=sink)))

    def test_v4_minimization_and_cuts_preserve_delivery_policy(self):
        self.check_minimization_and_cuts(4, 4)

    def test_v6_minimization_and_cuts_preserve_object_scope(self):
        self.check_minimization_and_cuts(6, 127)

    def test_v5_minimization_and_cuts_preserve_api_window_scope(self):
        self.check_minimization_and_cuts(5, 63)

    def check_minimization_and_cuts(self, version, mask):
        value = fixture()
        value.update(version=version, flight_capacity=1, flight_categories=mask,
                     flight_sink={"capacity": 1, "disconnect_before": 2})
        value["volume"]["tree_cache_pages"] = 2
        value["operations"][:0] = [
            {"op": "create", "label": "spare", "parent": "root", "name": "temp", "data": ""},
            {"op": "unlink", "label": "spare"}]
        value["expected"][0]["data"] = "ffff"
        records, success = tool.execute(tool.encoded(value), BINARY)
        self.assertFalse(success)
        signature = json.loads(tool.failure_signature(records))
        self.assertEqual(signature["flight_categories"], mask)
        self.assertEqual(signature["flight_sink"], value["flight_sink"])
        with tempfile.TemporaryDirectory(prefix="afsplus-selected-minimize-") as temporary:
            root = Path(temporary)
            tool.bundle.publish(root / "original", records)
            result = tool.minimize(root / "original", root / "reduced", BINARY)
            self.assertLess(result["operations"], len(value["operations"]))
            reduced = tool.bundle.read_bundle(root / "reduced")
            self.assertEqual(tool.failure_signature(reduced), tool.failure_signature(records))
            self.assertFalse(tool.replay(root / "reduced", BINARY))
        value["operations"] = [{"op": "create", "label": "f", "parent": "root", "name": "a", "data": "01"}]
        value["expected"] = []
        for variant in range(5):
            records, success = tool.execute(tool.encoded(value), BINARY,
                fault={"version": 1, "kind": "power-cut-v1", "operation": 0, "offset": 1, "variant": variant})
            self.assertTrue(success)
            tool.validate_trace(records)

    def test_v3_internal_flight_replays_all_profiles_and_reports_overwrite(self):
        for pages in (2, 4, 8, "unlimited"):
            for capacity in (1, 32):
                value = fixture()
                value.update(version=3, flight_capacity=capacity)
                value["volume"]["tree_cache_pages"] = pages
                records, success = tool.execute(tool.encoded(value), BINARY)
                self.assertTrue(success)
                flight = records["flight-recorder.bin"]
                self.assertEqual(flight[:8], b"AFSFLT02")
                self.assertEqual(struct.unpack_from("<I", flight, 12)[0], capacity)
                lost, count = struct.unpack_from("<QI", flight, 16 + 29)
                self.assertEqual((lost, count), (5, 1) if capacity == 1 else (0, 6))
                tool.validate_trace(records)
                with tempfile.TemporaryDirectory(prefix="afsplus-internal-flight-") as temporary:
                    path = Path(temporary) / "bundle"
                    tool.bundle.publish(path, records)
                    before = {p.name: p.read_bytes() for p in path.iterdir()}
                    result = self.cli("replay", path)
                    self.assertEqual(result.returncode, 0, result.stderr.decode())
                    self.assertEqual(before, {p.name: p.read_bytes() for p in path.iterdir()})

    def test_v3_internal_flight_rejects_resealed_corruption(self):
        value = fixture()
        value.update(version=3, flight_capacity=32)
        value["volume"]["tree_cache_pages"] = 2
        records, success = tool.execute(tool.encoded(value), BINARY)
        self.assertTrue(success)
        # Capacity, loss accounting, count, sequence, attempt, generation,
        # event kind and boolean are independently checked after re-sealing.
        first_event = 16 + 29 + 12
        changes = [(12, "I", 1), (16+29, "Q", 1), (16+29+8, "I", 257),
            (first_event, "Q", 2), (first_event+8, "Q", 0),
            (first_event+16, "Q", 0), (first_event+24, "B", 0),
            (first_event+25, "B", 2)]
        for offset, fmt, number in changes:
            flight = bytearray(records["flight-recorder.bin"])
            struct.pack_into("<"+fmt, flight, offset, number)
            edited = dict(records, **{"flight-recorder.bin": bytes(flight)})
            with self.assertRaisesRegex(ValueError, "flight"):
                tool.validate_trace(edited)
        for wire in (records["flight-recorder.bin"][:-1], records["flight-recorder.bin"]+b"x",
                     b"AFSFLT01"+records["flight-recorder.bin"][8:]):
            with self.assertRaisesRegex(ValueError, "flight"):
                tool.validate_trace(dict(records, **{"flight-recorder.bin": wire}))
        for capacity in (0, 257, True, "32"):
            value["flight_capacity"] = capacity
            with self.assertRaises(ValueError):
                tool.scenario.validate(tool.encoded(value))

    def test_v3_minimization_and_selected_cut_keep_diagnostic_policy(self):
        value = fixture()
        value.update(version=3, flight_capacity=1)
        value["volume"]["tree_cache_pages"] = 2
        value["operations"][:0] = [
            {"op": "create", "label": "spare", "parent": "root", "name": "temp", "data": ""},
            {"op": "unlink", "label": "spare"}]
        value["expected"][0]["data"] = "ffff"
        records, success = tool.execute(tool.encoded(value), BINARY)
        self.assertFalse(success)
        self.assertEqual(json.loads(tool.failure_signature(records))["flight_capacity"], 1)
        with tempfile.TemporaryDirectory(prefix="afsplus-flight-minimize-") as temporary:
            root = Path(temporary)
            tool.bundle.publish(root / "original", records)
            result = tool.minimize(root / "original", root / "reduced", BINARY)
            self.assertLess(result["operations"], len(value["operations"]))
            reduced = tool.bundle.read_bundle(root / "reduced")
            self.assertEqual(tool.failure_signature(reduced), tool.failure_signature(records))
            self.assertEqual(json.loads(reduced["operations.afstrace"])["flight_capacity"], 1)
            self.assertFalse(tool.replay(root / "reduced", BINARY))
        value["operations"] = [{"op": "create", "label": "f", "parent": "root", "name": "a", "data": "01"}]
        value["expected"] = []
        for variant in range(5):
            records, success = tool.execute(tool.encoded(value), BINARY,
                fault={"version": 1, "kind": "power-cut-v1", "operation": 0, "offset": 1, "variant": variant})
            self.assertTrue(success)
            tool.validate_trace(records)
            # The full recording is retained; a selected cut is not a claim
            # that these later commit events occurred on the cut device.
            self.assertEqual(records["flight-recorder.bin"][:8], b"AFSFLT02")

    def test_v2_ladder_profiles_survive_fresh_replay_without_artifact_writes(self):
        for pages in (2, 4, 8, "unlimited"):
            value = fixture()
            value["version"] = 2
            value["volume"]["tree_cache_pages"] = pages
            value["operations"] = [
                {"op": "mkdir", "label": "d", "parent": "root", "name": "src"},
                {"op": "create", "label": "f", "parent": "d", "name": "café", "data": "00ff"},
                {"op": "write", "label": "f", "offset": 4, "data": "42"},
                {"op": "truncate", "label": "f", "size": 2},
                {"op": "rename", "label": "f", "parent": "root", "name": "out"},
                {"op": "rmdir", "label": "d"},
                {"op": "create", "label": "spare", "parent": "root", "name": "temp", "data": ""},
                {"op": "unlink", "label": "spare"}, {"op": "sync"}, {"op": "remount"}]
            value["expected"] = [{"path": ["out"], "kind": "file", "data": "00ff"}]
            records, success = tool.execute(tool.encoded(value), BINARY)
            self.assertTrue(success)
            actual = json.loads(records["actual.json"])
            self.assertEqual(actual["version"], 3)
            self.assertEqual(actual["cache_pages"], pages)
            with tempfile.TemporaryDirectory(prefix="afsplus-profile-replay-") as temporary:
                path = Path(temporary) / "bundle"
                tool.bundle.publish(path, records)
                before = {p.name: p.read_bytes() for p in path.iterdir()}
                result = self.cli("replay", path)
                self.assertEqual(result.returncode, 0, result.stderr.decode())
                self.assertEqual(before, {p.name: p.read_bytes() for p in path.iterdir()})

    def test_v2_profile_binding_rejects_resealed_mismatches_and_silent_fallback(self):
        value = fixture()
        value["version"] = 2
        value["volume"]["tree_cache_pages"] = 2
        records, success = tool.execute(tool.encoded(value), BINARY)
        self.assertTrue(success)
        for change in ({"cache_pages": 4}, {"cache_pages": True}, {"version": 2}, {"cache_pages": "2"}):
            edited = dict(records)
            actual = json.loads(records["actual.json"])
            actual.update(change)
            edited["actual.json"] = tool.encoded(actual)
            with self.assertRaisesRegex(ValueError, "binding"):
                tool.validate_trace(edited)
        edited = dict(records)
        actual = json.loads(records["actual.json"])
        del actual["cache_pages"]
        edited["actual.json"] = tool.encoded(actual)
        with tempfile.TemporaryDirectory(prefix="afsplus-profile-mismatch-") as temporary:
            path = Path(temporary) / "resealed"
            tool.bundle.publish(path, edited)
            with self.assertRaisesRegex(ValueError, "binding"):
                tool.replay(path, BINARY)
        for text in ("02", "0", "true", "Unlimited"):
            with self.assertRaises(ValueError):
                tool.observation(f"AFSOBS03\ncache-pages {text}\nrun ok\nraw-check -\nrecovered-check -\nobserve ok\n".encode())

    def test_v2_minimization_keeps_profile_in_scenario_observation_and_signature(self):
        for pages in (2, 4, 8, "unlimited"):
            value = fixture()
            value["version"] = 2
            value["volume"]["tree_cache_pages"] = pages
            value["operations"][:0] = [
                {"op": "create", "label": "spare", "parent": "root", "name": "temp", "data": ""},
                {"op": "unlink", "label": "spare"}]
            value["expected"][0]["data"] = "ffff"
            records, success = tool.execute(tool.encoded(value), BINARY)
            self.assertFalse(success)
            self.assertEqual(json.loads(tool.failure_signature(records))["tree_cache_pages"], pages)
            other = dict(records)
            changed_value = json.loads(records["operations.afstrace"])
            changed_value["volume"]["tree_cache_pages"] = 4 if pages == 2 else 2
            changed_actual = json.loads(records["actual.json"])
            changed_actual["cache_pages"] = 4 if pages == 2 else 2
            other["operations.afstrace"] = tool.encoded(changed_value)
            other["actual.json"] = tool.encoded(changed_actual)
            self.assertNotEqual(tool.failure_signature(records), tool.failure_signature(other))
            with tempfile.TemporaryDirectory(prefix="afsplus-profile-minimize-") as temporary:
                root = Path(temporary)
                tool.bundle.publish(root / "original", records)
                result = tool.minimize(root / "original", root / "reduced", BINARY)
                self.assertLess(result["operations"], len(value["operations"]))
                reduced = tool.bundle.read_bundle(root / "reduced")
                self.assertEqual(json.loads(reduced["operations.afstrace"])["volume"]["tree_cache_pages"], pages)
                self.assertEqual(json.loads(reduced["actual.json"])["cache_pages"], pages)
                self.assertEqual(tool.failure_signature(reduced), tool.failure_signature(records))
                self.assertFalse(tool.replay(root / "reduced", BINARY))

    def test_v2_selected_crash_observation_uses_each_cache_profile(self):
        for pages in (2, 4, 8, "unlimited"):
            value = fixture()
            value["version"] = 2
            value["volume"]["tree_cache_pages"] = pages
            value["expected"] = []
            fault = {"version": 1, "kind": "power-cut-v1", "operation": 0, "offset": 1, "variant": 0}
            records, success = tool.execute(tool.encoded(value), BINARY, fault=fault)
            self.assertTrue(success)
            self.assertEqual(json.loads(records["actual.json"])["cache_pages"], pages)
            self.assertTrue(tool.verify_replay(records, BINARY, tool.bundle.DEFAULT_FILE_BYTES, tool.bundle.DEFAULT_TOTAL_BYTES))

    def cli(self, *args):
        return subprocess.run([sys.executable, str(Path(tool.__file__)), "--runner", str(BINARY), *map(str, args)],
            capture_output=True, timeout=120)

    def test_fresh_process_success_failure_and_original_immutability(self):
        with tempfile.TemporaryDirectory(prefix="afsplus-replay-") as temporary:
            root = Path(temporary)
            for wrong in (False, True):
                value = fixture()
                if wrong:
                    value["expected"][0]["data"] = "ffff"
                source = root / ("wrong.json" if wrong else "right.json")
                source.write_bytes(tool.encoded(value))
                output = root / ("failure" if wrong else "success")
                run = self.cli("run", source, output)
                self.assertEqual(run.returncode, 2 if wrong else 0, run.stderr)
                before = {p.name: p.read_bytes() for p in output.iterdir()}
                repeat = self.cli("replay", output)
                self.assertEqual(repeat.returncode, run.returncode, repeat.stderr)
                self.assertEqual(before, {p.name: p.read_bytes() for p in output.iterdir()})
                records = tool.bundle.read_bundle(output)
                tool.validate_trace(records)
                self.assertEqual(json.loads(records["run.json"])["outcome"], "failure" if wrong else "pass")

    def test_resealed_source_fault_and_base_mismatch_refuse(self):
        records, success = tool.execute(tool.encoded(fixture()), BINARY)
        self.assertTrue(success)
        with tempfile.TemporaryDirectory(prefix="afsplus-replay-invalid-") as temporary:
            for fault in ("source", "fault", "base", "flight", "trace"):
                edited = dict(records)
                if fault == "source":
                    meta = json.loads(edited["run.json"])
                    meta["source_observed"]["revision"] = "0" * 40
                    edited["run.json"] = tool.encoded(meta)
                elif fault == "fault":
                    edited["fault-model.json"] = tool.encoded({"version": 1, "kind": "arbitrary"})
                else:
                    role = {"base": "start.img", "flight": "flight-recorder.bin", "trace": "block-io.afstrace"}[fault]
                    data = bytearray(edited[role])
                    data[-1] ^= 1
                    edited[role] = bytes(data)
                path = Path(temporary) / fault
                tool.bundle.publish(path, edited)
                with self.assertRaises(ValueError):
                    tool.replay(path, BINARY)

    def test_operation_failure_is_preserved_as_failure_bundle(self):
        value = fixture()
        value["operations"].insert(1, {"op": "create", "label": "duplicate", "parent": "root", "name": "café", "data": ""})
        records, success = tool.execute(tool.encoded(value), BINARY)
        self.assertFalse(success)
        actual = json.loads(records["actual.json"])
        self.assertEqual(actual["failure"]["operation"], 1)
        self.assertEqual(actual["entries"][0]["data"], "00ff")
        tool.validate_trace(records)

    def test_minimizer_preserves_failure_and_original_with_explicit_budget(self):
        value = fixture()
        value["operations"][:0] = [
            {"op": "create", "label": "spare", "parent": "root", "name": "temp", "data": ""},
            {"op": "unlink", "label": "spare"}]
        value["expected"][0]["data"] = "ffff"
        records, success = tool.execute(tool.encoded(value), BINARY)
        self.assertFalse(success)
        with tempfile.TemporaryDirectory(prefix="afsplus-minimize-") as temporary:
            root = Path(temporary)
            original = root / "original"
            tool.bundle.publish(original, records)
            before = {p.name: p.read_bytes() for p in original.iterdir()}
            report = tool.minimize(original, root / "reduced", BINARY)
            self.assertFalse(report["budget_exhausted"])
            self.assertLess(report["operations"], len(value["operations"]))
            reduced = tool.bundle.read_bundle(root / "reduced")
            self.assertEqual(tool.failure_signature(reduced), tool.failure_signature(records))
            self.assertFalse(tool.replay(root / "reduced", BINARY))
            self.assertEqual(before, {p.name: p.read_bytes() for p in original.iterdir()})
            limited = tool.minimize(original, root / "limited", BINARY, max_runs=1)
            self.assertTrue(limited["budget_exhausted"])
            self.assertFalse(tool.replay(root / "limited", BINARY))

    def test_framed_output_rejects_truncation_roles_and_sizes(self):
        wire = bytearray(b"AFSRUN01")
        for role in tool.RUNNER_ROLES:
            raw = role.encode()
            wire.extend(struct.pack("<H", len(raw)) + raw + struct.pack("<Q", 0))
        self.assertEqual(set(tool.unframe(io.BytesIO(wire), 0, 0)), set(tool.RUNNER_ROLES))
        for stop in range(len(wire)):
            with self.assertRaises(ValueError):
                tool.unframe(io.BytesIO(wire[:stop]), 0, 0)
        for bad in (b"BADRUN01" + wire[8:], wire + b"x", wire[:10] + b"x" + wire[11:]):
            with self.assertRaises(ValueError):
                tool.unframe(io.BytesIO(bad), 0, 0)

    def test_selected_crashes_replay_and_minimize_with_a_stable_anchor(self):
        value = {"version": 1, "volume": fixture()["volume"],
            "operations": [{"op": "create", "label": "f", "parent": "root", "name": "a", "data": "01"}],
            "expected": []}
        with tempfile.TemporaryDirectory(prefix="afsplus-cut-") as temporary:
            root = Path(temporary)
            for variant in range(5):  # One unflushed write: loss/full plus three tears.
                fault = {"version": 1, "kind": "power-cut-v1", "operation": 0, "offset": 1, "variant": variant}
                records, success = tool.execute(tool.encoded(value), BINARY, fault=fault)
                self.assertTrue(success)
                tool.validate_trace(records)
                output = root / str(variant)
                tool.bundle.publish(output, records)
                result = self.cli("replay", output)
                self.assertEqual(result.returncode, 0, result.stderr)
            value["operations"][:0] = [
                {"op": "create", "label": "spare", "parent": "root", "name": "temp", "data": ""},
                {"op": "unlink", "label": "spare"}]
            value["operations"].append({"op": "sync"})
            value["expected"] = [{"path": ["missing"], "kind": "file", "data": ""}]
            fault = {"version": 1, "kind": "power-cut-v1", "operation": 2, "offset": 1, "variant": 0}
            records, success = tool.execute(tool.encoded(value), BINARY, fault=fault)
            self.assertFalse(success)
            original = root / "original"
            tool.bundle.publish(original, records)
            report = tool.minimize(original, root / "reduced", BINARY)
            self.assertFalse(report["budget_exhausted"])
            reduced = tool.bundle.read_bundle(root / "reduced")
            selected = json.loads(reduced["fault-model.json"])
            self.assertEqual(selected, dict(fault, operation=0))
            self.assertEqual(tool.failure_signature(records), tool.failure_signature(reduced))
            self.assertFalse(tool.replay(root / "reduced", BINARY))

    def test_durable_cut_keeps_committed_content_and_invalid_anchor_refuses(self):
        value = {"version": 1, "volume": fixture()["volume"],
            "operations": [{"op": "create", "label": "f", "parent": "root", "name": "a", "data": "01"}],
            "expected": [{"path": ["a"], "kind": "file", "data": "01"}]}
        records, success = tool.execute(tool.encoded(value), BINARY)
        self.assertTrue(success)
        _, first, last, _, _ = struct.unpack("<IQQQB", records["flight-recorder.bin"][12:41])
        fault = {"version": 1, "kind": "power-cut-v1", "operation": 0, "offset": last - first, "variant": 0}
        records, success = tool.execute(tool.encoded(value), BINARY, fault=fault)
        self.assertTrue(success)
        tool.validate_trace(records)
        for bad in (dict(fault, offset=65536), dict(fault, operation=True), dict(fault, variant=4132)):
            with self.assertRaises(ValueError):
                tool.execute(tool.encoded(value), BINARY, fault=bad)

    def test_checker_findings_are_required_and_preserved_in_the_failure_signature(self):
        records, success = tool.execute(tool.encoded(fixture()), BINARY)
        self.assertTrue(success)
        actual = json.loads(records["actual.json"])
        self.assertEqual(actual["version"], 2)
        self.assertTrue(tool.structural_success(actual))
        self.assertTrue(actual["raw_check"]["clean"])
        self.assertTrue(actual["recovered_check"]["clean"])
        for view in ("raw_check", "recovered_check"):
            damaged = json.loads(records["actual.json"])
            damaged[view]["clean"] = False
            damaged[view]["errors"] = ["reachable block marked free"]
            self.assertFalse(tool.structural_success(damaged))
            edited = dict(records, **{"actual.json": tool.encoded(damaged)})
            signature = json.loads(tool.failure_signature(edited))
            self.assertEqual(signature["kind"], "structure")
            self.assertEqual(signature["findings"][view], ["reachable block marked free"])
        report = actual["raw_check"]
        for field, value in (("schema_version", True), ("clean", 1), ("errors", ["hidden finding"])):
            edited = dict(report, **{field: value})
            with self.assertRaises(ValueError):
                tool.checker_report(tool.encoded(edited).hex())
        without_recovery = dict(actual, recovered_check=None)
        self.assertFalse(tool.structural_success(without_recovery))
        with self.assertRaises(ValueError):
            tool.observation(b"AFSOBS01\nrun ok\nobserve ok\n")

    def test_export_budget_refuses_before_runner(self):
        with self.assertRaisesRegex(ValueError, "per-file budget"):
            tool.execute(tool.encoded(fixture()), Path("/no/such/runner"), file_bytes=4096)
        with self.assertRaisesRegex(ValueError, "aggregate budget"):
            tool.execute(tool.encoded(fixture()), Path("/no/such/runner"), total_bytes=4096)


if __name__ == "__main__":
    unittest.main()
