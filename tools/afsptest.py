#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""Run and reproduce bounded private semantic bundles with cache and crash profiles."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import stat
import struct
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


bundle = module("bundle", "replay-bundle.py")
scenario = module("scenario", "replay-scenario.py")
FAULT = {"version": 1, "kind": "no-cut"}
RUNNER_ROLES = ("actual.wire", "flight-recorder.bin", "block-io.afstrace", "start.img", "result.img")


def encoded(value):
    return (json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True) + "\n").encode()


def digest(data):
    return hashlib.sha256(data).hexdigest()


def source_identity(root=ROOT):
    """Observed source identity, not an attestation of binary build provenance."""
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root).decode().strip()
    paths = subprocess.check_output(
        ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"], cwd=root).split(b"\0")
    tree = hashlib.sha256()
    for raw in sorted(set(paths) - {b""}):
        path = Path(root) / os.fsdecode(raw)
        tree.update(len(raw).to_bytes(4, "little") + raw)
        try:
            info = path.lstat()
        except FileNotFoundError:
            tree.update(b"missing")
            continue
        tree.update(str(stat.S_IMODE(info.st_mode)).encode() + b"\0")
        if stat.S_ISLNK(info.st_mode):
            tree.update(b"link" + hashlib.sha256(os.fsencode(os.readlink(path))).digest())
        elif stat.S_ISREG(info.st_mode):
            tree.update(b"file")
            content = hashlib.sha256()
            with path.open("rb") as stream:
                while chunk := stream.read(1024 * 1024):
                    content.update(chunk)
            tree.update(content.digest())
        else:
            raise ValueError("unsupported source identity file kind")
    return {"revision": revision, "working_tree_sha256": tree.hexdigest()}


def executable_digest(binary):
    with open(binary, "rb") as stream:
        value = hashlib.sha256()
        while chunk := stream.read(1024 * 1024):
            value.update(chunk)
        return value.hexdigest()


def exact(stream, count):
    result = stream.read(count)
    if len(result) != count:
        raise ValueError("truncated runner output")
    return result


def unframe(stream, file_bytes, total_bytes):
    if exact(stream, 8) != b"AFSRUN01":
        raise ValueError("runner output version")
    result = {}
    size = 0
    for expected in RUNNER_ROLES:
        length = struct.unpack("<H", exact(stream, 2))[0]
        if length > 64 or exact(stream, length).decode("ascii") != expected:
            raise ValueError("runner artifact role")
        count = struct.unpack("<Q", exact(stream, 8))[0]
        size += count
        if count > file_bytes or size > total_bytes:
            raise ValueError("runner artifact admission")
        result[expected] = exact(stream, count)
    if stream.read(1):
        raise ValueError("runner trailing output")
    return result


def checker_report(field, optional=False):
    if optional and field == "-":
        return None
    report = json.loads(bytes.fromhex(field), object_pairs_hook=bundle._unique)
    if not isinstance(report, dict) or set(report) != {"schema_version", "clean", "volume", "slots", "warnings", "errors"}:
        raise ValueError("checker report fields")
    if type(report["schema_version"]) is not int or report["schema_version"] != 5 or type(report["clean"]) is not bool:
        raise ValueError("checker report version/verdict")
    for key in ("slots", "warnings", "errors"):
        if not isinstance(report[key], list) or any(not isinstance(item, str) for item in report[key]):
            raise ValueError("checker diagnostic array")
    if report["clean"] != (not report["errors"]) or (report["clean"] and not isinstance(report["volume"], dict)):
        raise ValueError("checker verdict contradicts findings")
    return report


def structural_success(actual):
    return all(actual[key] is not None and actual[key]["clean"] for key in ("raw_check", "recovered_check"))


def observation(wire):
    lines = wire.decode("ascii").splitlines()
    cache = None
    if lines and lines[0] == "AFSOBS03":
        if len(lines) < 6 or lines[1] not in ("cache-pages 2", "cache-pages 4", "cache-pages 8", "cache-pages unlimited"):
            raise ValueError("observation cache profile")
        raw_cache = lines.pop(1).split(" ")[1]
        cache = raw_cache if raw_cache == "unlimited" else int(raw_cache)
        lines[0] = "AFSOBS02"
    if len(lines) < 5 or lines[0] != "AFSOBS02":
        raise ValueError("observation version")
    run = lines[1].split(" ")
    failure = None
    if run != ["run", "ok"]:
        if len(run) != 4 or run[:2] != ["run", "error"]:
            raise ValueError("run outcome")
        failure = {"operation": int(run[2]), "error": bytes.fromhex(run[3]).decode()}
    raw = lines[2].split(" ")
    recovered = lines[3].split(" ")
    if len(raw) != 2 or raw[0] != "raw-check" or len(recovered) != 2 or recovered[0] != "recovered-check":
        raise ValueError("checker observation fields")
    raw_check = checker_report(raw[1])
    recovered_check = checker_report(recovered[1], optional=True)
    observe = lines[4].split(" ")
    error = None
    if observe != ["observe", "ok"]:
        if len(observe) != 3 or observe[:2] != ["observe", "error"] or len(lines) != 5:
            raise ValueError("observation outcome")
        error = bytes.fromhex(observe[2]).decode()
    entries = []
    for line in lines[5:]:
        fields = line.split(" ")
        if len(fields) not in (2, 3) or fields[0] not in ("file", "directory"):
            raise ValueError("observation entry")
        entry = {"kind": fields[0], "path": [bytes.fromhex(p).decode() for p in fields[1].split(",")]}
        if fields[0] == "file":
            if len(fields) != 3:
                raise ValueError("missing observed contents")
            entry["data"] = "" if fields[2] == "-" else bytes.fromhex(fields[2]).hex()
        elif len(fields) != 2:
            raise ValueError("directory payload")
        entries.append(entry)
    result = {"version": 2, "view": "remounted", "failure": failure, "inspection_error": error,
        "raw_check": raw_check, "recovered_check": recovered_check, "entries": entries}
    if cache is not None:
        result.update(version=3, cache_pages=cache)
    return result


def bind_cache_profile(value, actual):
    """Admitted configuration and observed policy must agree before replay."""
    if not isinstance(actual, dict):
        raise ValueError("observation object")
    expected_version = 3 if value["version"] >= 2 else 2
    if type(actual.get("version")) is not int or actual["version"] != expected_version:
        raise ValueError("scenario/observation profile version binding")
    if value["version"] >= 2:
        expected = value["volume"]["tree_cache_pages"]
        observed = actual.get("cache_pages")
        if type(observed) is not type(expected) or observed != expected:
            raise ValueError("scenario/observation cache profile binding")
    elif "cache_pages" in actual:
        raise ValueError("version-1 scenario cannot declare a cache profile")


def admit_fault(fault, value):
    if not isinstance(fault, dict) or type(fault.get("version")) is not int or fault["version"] != 1:
        raise ValueError("fault model version")
    if fault == FAULT:
        return fault
    if set(fault) != {"version", "kind", "operation", "offset", "variant"} or fault["kind"] != "power-cut-v1":
        raise ValueError("unsupported fault model")
    scenario.integer(fault["operation"], 0, len(value["operations"]) - 1)
    scenario.integer(fault["offset"], 0, 65536)
    scenario.integer(fault["variant"], 0, 4131)
    return fault


def execute(raw, binary, file_bytes=bundle.DEFAULT_FILE_BYTES, total_bytes=bundle.DEFAULT_TOTAL_BYTES, fault=None, *, source_root=ROOT):
    value = scenario.validate(raw)
    fault = admit_fault(FAULT if fault is None else fault, value)
    # Admit image exports before starting the runner.
    image_bytes = value["volume"]["blocks"] * value["volume"]["block_size"]
    if image_bytes > file_bytes:
        raise ValueError("image export exceeds per-file budget")
    if 2 * image_bytes > total_bytes:
        raise ValueError("image exports exceed aggregate budget")
    commands = scenario.compile_commands(raw)
    if fault["kind"] == "power-cut-v1":
        commands = ("AFSCUT01 {operation} {offset} {variant}\n".format(**fault)).encode() + commands
    identity = source_identity(source_root)
    binary_hash = executable_digest(binary)
    # Runner output is intrinsically bounded by its profile; temporary spool
    # avoids an unbounded subprocess.PIPE allocation. It is never a caller image.
    with tempfile.TemporaryFile() as output, tempfile.TemporaryFile() as errors:
        result = subprocess.run([str(binary)], input=commands, stdout=output, stderr=errors, timeout=120)
        if result.returncode:
            errors.seek(0)
            raise ValueError("runner failed: " + errors.read(4096).decode(errors="replace"))
        output.seek(0)
        records = unframe(output, file_bytes, total_bytes)
    if source_identity(source_root) != identity or executable_digest(binary) != binary_hash:
        raise ValueError("source or runner changed during execution")
    actual = observation(records.pop("actual.wire"))
    bind_cache_profile(value, actual)
    expected = sorted(value["expected"], key=lambda entry: entry["path"])
    success = (actual["failure"] is None and actual["inspection_error"] is None
               and structural_success(actual) and actual["entries"] == expected)
    records.update({
        "operations.afstrace": raw,
        "run.json": encoded({"version": 1, "profile": "semantic-no-cut-v1" if fault["kind"] == "no-cut" else "semantic-power-cut-v1",
            "source_observed": identity, "runner_sha256": binary_hash, "outcome": "pass" if success else "failure"}),
        "fault-model.json": encoded(fault),
        "expected.json": encoded(expected),
        "actual.json": encoded(actual),
    })
    validate_trace(records)
    return records, success


def selected_batch(flight, offset, previous, capacity, profile, index):
    extended = profile["version"] == 5
    if len(flight) - offset < 53:
        raise ValueError("flight truncated selected batch")
    lost, filtered, sequence, attempt, delivered, missed, closed, retained = struct.unpack(
        "<QQQQQQBI", flight[offset:offset + 53])
    offset += 53
    old_sequence, old_attempt, old_lost, old_filtered, old_delivered, old_missed, old_closed = previous
    if (sequence < old_sequence or attempt < old_attempt or lost < old_lost
            or filtered < old_filtered or delivered < old_delivered or missed < old_missed
            or closed not in (0, 1) or attempt > sequence
            or attempt - old_attempt > sequence - old_sequence
            or (not extended and (sequence == 0) != (attempt == 0))):
        raise ValueError("flight selected counters or identities")
    selected = sequence - old_sequence - (filtered - old_filtered)
    if (selected < 0 or retained != min(capacity, selected)
            or lost - old_lost != selected - retained):
        raise ValueError("flight filtered/lost conservation")
    sink = profile["flight_sink"]
    if sink is None:
        expected_delivered = expected_missed = expected_closed = 0
    else:
        disconnected = sink["disconnect_before"] is not None and index >= sink["disconnect_before"]
        accepted = 0 if disconnected else min(sink["capacity"], selected)
        expected_delivered = old_delivered + accepted
        expected_missed = old_missed + selected - accepted
        expected_closed = bool(old_closed or (disconnected and selected))
    if (delivered, missed, closed) != (expected_delivered, expected_missed, expected_closed):
        raise ValueError("flight live delivery differs from deterministic profile")
    event_size = 64 if extended else 26
    if len(flight) - offset < retained * event_size:
        raise ValueError("flight truncated selected event")
    cursor, observed_attempt = old_sequence, old_attempt
    categories = {1: 1, 2: 4, 3: 2, 4: 2, 5: 2, 6: 1, 7: 8}
    if extended:
        categories.update({kind: 16 for kind in range(8, 12)})
        categories.update({kind: 32 for kind in range(12, 20)})
    contexts = {}
    for ordinal in range(retained):
        seq, tx, generation, kind, remount = struct.unpack("<QQQBB", flight[offset:offset + 26])
        offset += 26
        uncommitted_event = extended and kind >= 8 and tx == 0
        if (not cursor < seq <= sequence
                or (not uncommitted_event and not observed_attempt <= tx <= attempt)
                or (tx == 0 and (not extended or kind < 8)) or tx > seq or generation == 0 or kind not in categories
                or not profile["flight_categories"] & categories.get(kind, 0)
                or remount not in (0, 1) or (kind == 6 and remount)
                or (kind == 1 and tx == observed_attempt)):
            raise ValueError("flight selected identity/category/event")
        if ordinal == 0 and seq - old_sequence - 1 < lost - old_lost:
            raise ValueError("flight overwritten events are not a prefix")
        if extended:
            operation, span, parent, method, window, group = struct.unpack(
                "<QQQHQI", flight[offset:offset + 38])
            offset += 38
            if ((span == 0 and (operation or parent or method))
                    or (span and (not 1 <= method <= 66 or not 0 < operation <= span <= seq
                                  or parent >= span or (parent == 0 and operation != span)
                                  or (parent and operation > parent)))
                    or (8 <= kind <= 11 and span == 0)
                    or (kind >= 8 and tx != 0)
                    or window > seq or (window == 0 and group != 0)
                    or (kind >= 12 and window == 0)
                    or (kind in (14, 15, 16) and group == 0)):
                raise ValueError("flight API/window context")
            if span:
                context = (operation, parent, method)
                if span in contexts and contexts[span] != context:
                    raise ValueError("flight reused span context")
                contexts[span] = context
        cursor = seq
        observed_attempt = max(observed_attempt, tx)
    return offset, (sequence, attempt, lost, filtered, delivered, missed, closed)


def validate_trace(records):
    """Independent wire/base/result admission before semantic execution."""
    scenario_value = scenario.validate(records["operations.afstrace"])
    bind_cache_profile(scenario_value, json.loads(records["actual.json"], object_pairs_hook=scenario.unique))
    wire = records["block-io.afstrace"]
    if len(wire) < 96 or wire[:8] != b"AFSTRC00" or hashlib.sha256(wire[:-32]).digest() != wire[-32:]:
        raise ValueError("block trace integrity")
    version, bs, blocks, count = struct.unpack("<IIQQ", wire[8:32])
    if version != 1 or bs != 4096 or not 64 <= blocks <= 65536 or count > 65536:
        raise ValueError("block trace geometry/count")
    base = records["start.img"]
    if len(base) != bs * blocks or len(records["result.img"]) != len(base):
        raise ValueError("image geometry binding")
    value = hashlib.sha256(b"AFS+ replay base v1\0" + struct.pack("<IQ", bs, blocks))
    value.update(base)
    if value.digest() != wire[32:64]:
        raise ValueError("base identity binding")
    result = bytearray(base)
    operations = []
    offset = 64
    for _ in range(count):
        if offset >= len(wire) - 32:
            raise ValueError("trace record count")
        tag = wire[offset]
        offset += 1
        if tag == 1:
            if offset + 8 + bs > len(wire) - 32:
                raise ValueError("truncated block operation")
            lba = struct.unpack("<Q", wire[offset:offset + 8])[0]
            if lba >= blocks:
                raise ValueError("trace LBA")
            offset += 8
            result[lba * bs:(lba + 1) * bs] = wire[offset:offset + bs]
            operations.append((lba, offset))
            offset += bs
        elif tag == 2:
            operations.append(None)
        else:
            raise ValueError("trace operation tag")
    if offset != len(wire) - 32:
        raise ValueError("block trace trailing records")
    flight = records["flight-recorder.bin"]
    internal = scenario_value["version"] >= 3
    selected = scenario_value["version"] >= 4
    magic = b"AFSFLT04" if scenario_value["version"] == 5 else b"AFSFLT03" if selected else b"AFSFLT02" if internal else b"AFSFLT01"
    if len(flight) < 12 or flight[:8] != magic:
        raise ValueError("flight version")
    events = struct.unpack("<I", flight[8:12])[0]
    if events > 1024 or events > len(scenario_value["operations"]):
        raise ValueError("flight event admission")
    offset = 12
    capacity = None
    if internal:
        if len(flight) < 16:
            raise ValueError("flight capacity header")
        capacity = struct.unpack("<I", flight[12:16])[0]
        if capacity != scenario_value["flight_capacity"]:
            raise ValueError("flight capacity differs from scenario")
        offset = 16
    if selected:
        if len(flight) < 28:
            raise ValueError("flight selected profile header")
        categories, sink_capacity, disconnect = struct.unpack("<III", flight[16:28])
        sink = scenario_value["flight_sink"]
        expected_sink = 0 if sink is None else sink["capacity"]
        expected_disconnect = None if sink is None else sink["disconnect_before"]
        expected_disconnect = 0xffffffff if expected_disconnect is None else expected_disconnect
        if (categories, sink_capacity, disconnect) != (scenario_value["flight_categories"], expected_sink, expected_disconnect):
            raise ValueError("flight selected profile differs from scenario")
        offset = 28
    end = 0
    ranges = []
    selected_state = (0, 0, 0, 0, 0, 0, 0)
    sequence = attempt = dropped = 0
    for index in range(events):
        if len(flight) - offset < 29:
            raise ValueError("flight truncated operation")
        operation, first, last, _, success = struct.unpack("<IQQQB", flight[offset:offset + 29])
        offset += 29
        if operation != index or not end <= first <= last <= count or success not in (0, 1):
            raise ValueError("flight block range")
        if not success and index != events - 1:
            raise ValueError("flight continued past failure")
        end = last
        ranges.append((first, last))
        if selected:
            offset, selected_state = selected_batch(flight, offset, selected_state, capacity, scenario_value, index)
        elif internal:
            if len(flight) - offset < 12:
                raise ValueError("flight truncated batch")
            lost, retained = struct.unpack("<QI", flight[offset:offset + 12])
            offset += 12
            if retained > capacity or lost < dropped or (lost > dropped and retained != capacity):
                raise ValueError("flight loss accounting")
            if len(flight) - offset < retained * 26:
                raise ValueError("flight truncated internal event")
            sequence += lost - dropped
            dropped = lost
            for _ in range(retained):
                seq, tx, generation, kind, remount = struct.unpack("<QQQBB", flight[offset:offset + 26])
                offset += 26
                if (seq != sequence + 1 or tx < attempt or tx == 0 or tx > seq
                        or generation == 0 or kind not in range(1, 8) or remount not in (0, 1)
                        or (kind == 6 and remount)):
                    raise ValueError("flight internal identity or event")
                sequence, attempt = seq, tx
    if offset != len(flight):
        raise ValueError("flight trailing records")
    fault = admit_fault(json.loads(records["fault-model.json"], object_pairs_hook=bundle._unique),
                        scenario_value)
    selection = None
    if fault["kind"] == "power-cut-v1":
        if fault["operation"] >= len(ranges):
            raise ValueError("fault operation has no event")
        first, last = ranges[fault["operation"]]
        cut = first + fault["offset"]
        if cut > last:
            raise ValueError("fault offset outside event")
        durable = max((i + 1 for i, op in enumerate(operations[:cut]) if op is None), default=0)
        tail = operations[durable:cut]
        if len(tail) > 12:
            raise ValueError("fault tail exceeds model")
        subsets = 1 << len(tail)
        tears = (64, 2048, 4064)
        variant = fault["variant"]
        if variant >= subsets + len(tail) * len(tears):
            raise ValueError("fault variant outside model")
        selection = {"tail_writes": len(tail), "variant": variant,
                     "kind": "subset" if variant < subsets else "tear"}
        result = bytearray(base)
        def apply(op, length=bs):
            if op is not None:
                lba, start = op
                result[lba * bs:lba * bs + length] = wire[start:start + length]
        for op in operations[:durable]:
            apply(op)
        if variant < subsets:
            for i, op in enumerate(tail):
                if variant & (1 << i):
                    apply(op)
        else:
            selected, tear = divmod(variant - subsets, len(tears))
            for op in tail[:selected]:
                apply(op)
            apply(tail[selected], tears[tear])
    if result != records["result.img"]:
        raise ValueError("block trace result binding")
    return selection


def admit_run(retained):
    """Validate historical metadata independently of the selected executable."""
    meta = json.loads(retained["run.json"], object_pairs_hook=bundle._unique)
    required = {"version", "profile", "source_observed", "runner_sha256", "outcome"}
    if not isinstance(meta, dict) or set(meta) not in (required, required | {"reduction"}):
        raise ValueError("run metadata fields")
    reduction = meta.get("reduction")
    if "reduction" in meta:
        if not isinstance(reduction, dict) or set(reduction) != {"parent_scenario_sha256", "evaluations", "budget_exhausted"}:
            raise ValueError("reduction metadata")
        parent = reduction["parent_scenario_sha256"]
        if not isinstance(parent, str) or len(parent) != 64 or any(c not in "0123456789abcdef" for c in parent):
            raise ValueError("reduction parent digest")
        if type(reduction["evaluations"]) is not int or not 0 <= reduction["evaluations"] <= 4096 or type(reduction["budget_exhausted"]) is not bool:
            raise ValueError("reduction budget")
    fault = admit_fault(json.loads(retained["fault-model.json"], object_pairs_hook=bundle._unique),
                        scenario.validate(retained["operations.afstrace"]))
    profile = "semantic-no-cut-v1" if fault["kind"] == "no-cut" else "semantic-power-cut-v1"
    if type(meta["version"]) is not int or meta["version"] != 1 or meta["profile"] != profile:
        raise ValueError("run profile")
    source = meta["source_observed"]
    def hex_digest(value, lengths):
        return isinstance(value, str) and len(value) in lengths and all(c in "0123456789abcdef" for c in value)
    if (not isinstance(source, dict) or set(source) != {"revision", "working_tree_sha256"}
            or not hex_digest(source["revision"], (40, 64))
            or not hex_digest(source["working_tree_sha256"], (64,))
            or not hex_digest(meta["runner_sha256"], (64,))):
        raise ValueError("run source or runner identity schema")
    value = scenario.validate(retained["operations.afstrace"])
    expected = sorted(value["expected"], key=lambda entry: entry["path"])
    if retained["expected.json"] != encoded(expected):
        raise ValueError("scenario/expected state binding")
    actual = json.loads(retained["actual.json"], object_pairs_hook=bundle._unique)
    if not isinstance(actual, dict) or not {"failure", "inspection_error", "raw_check", "recovered_check", "entries"} <= set(actual):
        raise ValueError("run observation fields")
    clean = all(isinstance(actual[key], dict) and actual[key].get("clean") is True
                for key in ("raw_check", "recovered_check"))
    success = actual["failure"] is None and actual["inspection_error"] is None and clean and actual["entries"] == expected
    if meta["outcome"] != ("pass" if success else "failure"):
        raise ValueError("run outcome contradicts observation")
    validate_trace(retained)
    return meta, fault


def verify_replay(retained, binary, file_bytes, total_bytes):
    # Neither runner path nor commands are selected by bundle metadata.
    meta, fault = admit_run(retained)
    reduction = meta.get("reduction")
    if meta["source_observed"] != source_identity() or meta["runner_sha256"] != executable_digest(binary):
        raise ValueError("source or runner identity mismatch")
    regenerated, success = execute(retained["operations.afstrace"], binary, file_bytes, total_bytes, fault)
    if reduction is not None:
        regenerated_meta = json.loads(regenerated["run.json"])
        regenerated_meta["reduction"] = reduction
        regenerated["run.json"] = encoded(regenerated_meta)
    for role in sorted(bundle.ROLES):
        if regenerated[role] != retained[role]:
            raise ValueError("semantic replay mismatch: " + role)
    return success


def replay(path, binary, file_bytes=bundle.DEFAULT_FILE_BYTES, total_bytes=bundle.DEFAULT_TOTAL_BYTES):
    retained = bundle.read_bundle(path, file_bytes, total_bytes)
    return verify_replay(retained, binary, file_bytes, total_bytes)


def compare_rebuilt(path, output, binary, source_root, file_bytes=bundle.DEFAULT_FILE_BYTES,
                    total_bytes=bundle.DEFAULT_TOTAL_BYTES):
    """Compare a caller-selected rebuilt runner; never waive exact replay binding.

    Both complete bundles are retained beneath a fresh directory. report.json is
    its completion record, published only after the nested bundles are durable.
    This observes source and binary identities, not build provenance.
    """
    retained = bundle.read_bundle(path, file_bytes, total_bytes)
    original_meta, fault = admit_run(retained)
    if source_identity(source_root) != original_meta["source_observed"]:
        raise ValueError("rebuilt comparison source identity mismatch")
    output = Path(output)
    # Do not put generated evidence into either observed source tree or the input
    # bundle. Such output would contaminate its own source identity or originals.
    destination = output.resolve()
    for protected in (Path(source_root).resolve(), Path(path).resolve()):
        if destination == protected or protected in destination.parents:
            raise ValueError("comparison output overlaps source or original bundle")
    if os.path.lexists(output):
        raise ValueError("comparison output already exists")
    regenerated, success = execute(retained["operations.afstrace"], binary, file_bytes,
                                   total_bytes, fault, source_root=source_root)
    rebuilt_meta, _ = admit_run(regenerated)
    if rebuilt_meta["source_observed"] != original_meta["source_observed"]:
        raise ValueError("rebuilt comparison source changed before execution")
    roles = sorted(bundle.ROLES - {"run.json"})
    differences = [role for role in roles if retained[role] != regenerated[role]]
    report = {"version": 1, "kind": "rebuilt-semantic-comparison-v1",
        "source_observed": original_meta["source_observed"],
        "original_runner_sha256": original_meta["runner_sha256"],
        "rebuilt_runner_sha256": rebuilt_meta["runner_sha256"],
        "runner_identical": original_meta["runner_sha256"] == rebuilt_meta["runner_sha256"],
        "original_outcome": original_meta["outcome"], "rebuilt_outcome": rebuilt_meta["outcome"],
        "semantic_artifacts_equal": not differences, "differing_roles": differences,
        "build_provenance_attested": False,
        "artifacts": [{"role": role, "original_sha256": digest(retained[role]),
                       "rebuilt_sha256": digest(regenerated[role])} for role in sorted(bundle.ROLES)]}
    os.mkdir(output, 0o700)
    directory = bundle._directory(output)
    try:
        bundle.publish(output / "original", retained, file_bytes, total_bytes)
        bundle.publish(output / "rebuilt", regenerated, file_bytes, total_bytes)
        bundle._write(directory, "report.pending", encoded(report))
        os.link("report.pending", "report.json", src_dir_fd=directory,
                dst_dir_fd=directory, follow_symlinks=False)
        os.unlink("report.pending", dir_fd=directory)
        os.fsync(directory)
        parent = bundle._directory(output.parent)
        try:
            os.fsync(parent)
        finally:
            os.close(parent)
    finally:
        os.close(directory)
    return report, success


def failure_signature(records):
    actual = json.loads(records["actual.json"])
    value = scenario.validate(records["operations.afstrace"])
    bind_cache_profile(value, actual)
    def pack(signature):
        if value["version"] >= 2:
            signature["tree_cache_pages"] = value["volume"]["tree_cache_pages"]
        if value["version"] >= 3:
            signature["flight_capacity"] = value["flight_capacity"]
        if value["version"] >= 4:
            signature["flight_categories"] = value["flight_categories"]
            signature["flight_sink"] = value["flight_sink"]
        return encoded(signature)
    if actual["failure"] is not None:
        failure = actual["failure"]
        operations = value["operations"]
        index = failure["operation"]
        if not 0 <= index < len(operations):
            raise ValueError("failure has no semantic operation")
        return pack({"kind": "operation", "operation": operations[index], "error": failure["error"]})
    structural = {key: None if actual[key] is None else actual[key]["errors"]
                  for key in ("raw_check", "recovered_check")}
    if not structural_success(actual):
        return pack({"kind": "structure", "findings": structural,
                        "inspection_error": actual["inspection_error"]})
    if actual["inspection_error"] is not None:
        return pack({"kind": "inspection", "error": actual["inspection_error"]})
    expected = {tuple(entry["path"]): entry for entry in json.loads(records["expected.json"])}
    observed = {tuple(entry["path"]): entry for entry in actual["entries"]}
    differences = [{"path": list(path), "expected": expected.get(path), "actual": observed.get(path)}
        for path in sorted(set(expected) | set(observed)) if expected.get(path) != observed.get(path)]
    if not differences:
        raise ValueError("minimization requires a reproducing failure")
    return pack({"kind": "state", "differences": differences})


def minimize(path, output, binary, max_runs=128, file_bytes=bundle.DEFAULT_FILE_BYTES,
             total_bytes=bundle.DEFAULT_TOTAL_BYTES):
    if type(max_runs) is not int or not 1 <= max_runs <= 4096:
        raise ValueError("minimizer execution budget")
    original = bundle.read_bundle(path, file_bytes, total_bytes)
    if verify_replay(original, binary, file_bytes, total_bytes):
        raise ValueError("cannot minimize a successful run")
    signature = failure_signature(original)
    selection = validate_trace(original)
    best = original
    value = scenario.validate(original["operations.afstrace"])
    fault = json.loads(original["fault-model.json"])
    runs = 0
    granularity = 2
    exhausted = False
    while value["operations"]:
        count = len(value["operations"])
        chunk = (count + granularity - 1) // granularity
        reduced = False
        for start in range(0, count, chunk):
            candidate_fault = dict(fault)
            if fault["kind"] == "power-cut-v1":
                anchor = fault["operation"]
                if start <= anchor < min(start + chunk, count):
                    continue
                if anchor >= start + chunk:
                    candidate_fault["operation"] -= chunk
            candidate = dict(value)
            candidate["operations"] = value["operations"][:start] + value["operations"][start + chunk:]
            raw = encoded(candidate)
            try:
                scenario.validate(raw)
            except ValueError:
                continue
            if runs >= max_runs:
                exhausted = True
                break
            runs += 1
            try:
                records, success = execute(raw, binary, file_bytes, total_bytes, candidate_fault)
            except ValueError as error:
                # An altered operation may no longer contain the selected cut.
                # Reject that candidate; never substitute its admission failure.
                refusals = {"fault offset outside operation", "crash variant outside model",
                            "crash tail exceeds exhaustive model", "fault scenario did not finish recording"}
                if str(error).strip() in {"runner failed: scenario refused: " + reason for reason in refusals}:
                    continue
                raise
            if not success and validate_trace(records) == selection and failure_signature(records) == signature:
                value, best, fault = candidate, records, candidate_fault
                granularity = max(2, granularity - 1)
                reduced = True
                break
        if exhausted:
            break
        if not reduced:
            if granularity >= count:
                break
            granularity = min(count, granularity * 2)
    meta = json.loads(best["run.json"])
    meta["reduction"] = {"parent_scenario_sha256": digest(original["operations.afstrace"]),
        "evaluations": runs, "budget_exhausted": exhausted}
    best = dict(best)
    best["run.json"] = encoded(meta)
    bundle.publish(output, best, file_bytes, total_bytes)
    return {"operations": len(value["operations"]), "evaluations": runs, "budget_exhausted": exhausted}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runner", type=Path, default=ROOT / "target/debug/afsplus-scenario")
    parser.add_argument("--file-bytes", type=int, default=bundle.DEFAULT_FILE_BYTES)
    parser.add_argument("--total-bytes", type=int, default=bundle.DEFAULT_TOTAL_BYTES)
    commands = parser.add_subparsers(dest="command", required=True)
    run = commands.add_parser("run")
    run.add_argument("scenario", type=Path)
    run.add_argument("output", type=Path)
    run.add_argument("--fault", type=Path, help="versioned no-cut or anchored power-cut JSON")
    repeat = commands.add_parser("replay")
    repeat.add_argument("bundle", type=Path)
    shrink = commands.add_parser("minimize")
    shrink.add_argument("bundle", type=Path)
    shrink.add_argument("output", type=Path)
    shrink.add_argument("--max-runs", type=int, default=128)
    compare = commands.add_parser("compare-rebuilt")
    compare.add_argument("bundle", type=Path)
    compare.add_argument("output", type=Path)
    compare.add_argument("--source-root", type=Path, required=True,
                         help="caller-selected checkout matching the original source identity")
    args = parser.parse_args()
    try:
        if args.command == "compare-rebuilt":
            report, success = compare_rebuilt(args.bundle, args.output, args.runner,
                args.source_root, args.file_bytes, args.total_bytes)
            print("rebuilt comparison: " + encoded(report).decode().strip())
            raise SystemExit(0 if report["semantic_artifacts_equal"] and success else 2)
        if args.command == "minimize":
            report = minimize(args.bundle, args.output, args.runner, args.max_runs, args.file_bytes, args.total_bytes)
            print("failure reduction: " + encoded(report).decode().strip())
            return
        if args.command == "run":
            directory = bundle._directory(args.scenario.parent)
            try:
                raw = bundle._read(directory, args.scenario.name, scenario.MAX_INPUT)
            finally:
                os.close(directory)
            fault = None
            if args.fault is not None:
                directory = bundle._directory(args.fault.parent)
                try:
                    fault = json.loads(bundle._read(directory, args.fault.name, 4096), object_pairs_hook=bundle._unique)
                finally:
                    os.close(directory)
            records, success = execute(raw, args.runner, args.file_bytes, args.total_bytes, fault)
            bundle.publish(args.output, records, args.file_bytes, args.total_bytes)
        else:
            success = replay(args.bundle, args.runner, args.file_bytes, args.total_bytes)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        parser.exit(1, f"replay refused: {error}\n")
    print(("run" if args.command == "run" else "replay reproduced") + (": pass" if success else ": semantic failure"))
    raise SystemExit(0 if success else 2)


if __name__ == "__main__":
    main()
