#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""Run and reproduce bounded private semantic bundles (no-cut profile)."""
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


def source_identity():
    """Observed source identity, not an attestation of binary build provenance."""
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT).decode().strip()
    paths = subprocess.check_output(
        ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"], cwd=ROOT).split(b"\0")
    tree = hashlib.sha256()
    for raw in sorted(set(paths) - {b""}):
        path = ROOT / os.fsdecode(raw)
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
    return {"version": 2, "view": "remounted", "failure": failure, "inspection_error": error,
        "raw_check": raw_check, "recovered_check": recovered_check, "entries": entries}


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


def execute(raw, binary, file_bytes=bundle.DEFAULT_FILE_BYTES, total_bytes=bundle.DEFAULT_TOTAL_BYTES, fault=None):
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
    identity = source_identity()
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
    if source_identity() != identity or executable_digest(binary) != binary_hash:
        raise ValueError("source or runner changed during execution")
    actual = observation(records.pop("actual.wire"))
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


def validate_trace(records):
    """Independent wire/base/result admission before semantic execution."""
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
    if len(flight) < 12 or flight[:8] != b"AFSFLT01":
        raise ValueError("flight version")
    events = struct.unpack("<I", flight[8:12])[0]
    if events > 1024 or len(flight) != 12 + 29 * events:
        raise ValueError("flight event admission")
    end = 0
    ranges = []
    for index in range(events):
        operation, first, last, _, success = struct.unpack("<IQQQB", flight[12 + 29 * index:41 + 29 * index])
        if operation != index or not end <= first <= last <= count or success not in (0, 1):
            raise ValueError("flight block range")
        if not success and index != events - 1:
            raise ValueError("flight continued past failure")
        end = last
        ranges.append((first, last))
    fault = admit_fault(json.loads(records["fault-model.json"], object_pairs_hook=bundle._unique),
                        scenario.validate(records["operations.afstrace"]))
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


def verify_replay(retained, binary, file_bytes, total_bytes):
    # Neither runner path nor commands are selected by bundle metadata.
    meta = json.loads(retained["run.json"], object_pairs_hook=bundle._unique)
    required = {"version", "profile", "source_observed", "runner_sha256", "outcome"}
    if not isinstance(meta, dict) or set(meta) not in (required, required | {"reduction"}):
        raise ValueError("run metadata fields")
    reduction = meta.get("reduction")
    if reduction is not None:
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
    if meta["source_observed"] != source_identity() or meta["runner_sha256"] != executable_digest(binary):
        raise ValueError("source or runner identity mismatch")
    validate_trace(retained)
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


def failure_signature(records):
    actual = json.loads(records["actual.json"])
    if actual["failure"] is not None:
        failure = actual["failure"]
        operations = scenario.validate(records["operations.afstrace"])["operations"]
        index = failure["operation"]
        if not 0 <= index < len(operations):
            raise ValueError("failure has no semantic operation")
        return encoded({"kind": "operation", "operation": operations[index], "error": failure["error"]})
    structural = {key: None if actual[key] is None else actual[key]["errors"]
                  for key in ("raw_check", "recovered_check")}
    if not structural_success(actual):
        return encoded({"kind": "structure", "findings": structural,
                        "inspection_error": actual["inspection_error"]})
    if actual["inspection_error"] is not None:
        return encoded({"kind": "inspection", "error": actual["inspection_error"]})
    expected = {tuple(entry["path"]): entry for entry in json.loads(records["expected.json"])}
    observed = {tuple(entry["path"]): entry for entry in actual["entries"]}
    differences = [{"path": list(path), "expected": expected.get(path), "actual": observed.get(path)}
        for path in sorted(set(expected) | set(observed)) if expected.get(path) != observed.get(path)]
    if not differences:
        raise ValueError("minimization requires a reproducing failure")
    return encoded({"kind": "state", "differences": differences})


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
    args = parser.parse_args()
    try:
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
