#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""Seeded namespace/content properties through the bounded semantic replay runner."""
import argparse
import importlib.util
import json
import os
import subprocess
from pathlib import Path

spec = importlib.util.spec_from_file_location("semantic_properties_runner", Path(__file__).with_name("afsptest.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)
VERSION = 1
MASK = (1 << 64) - 1
PROFILES = (2, 4, 8, "unlimited")


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


def qualify(output, binary, seeds, steps, bundle_payload_bytes=512 * 1024 * 1024):
    runner.scenario.integer(steps, 64, 256)
    runner.scenario.integer(bundle_payload_bytes, 1, 4 * 1024**3)
    if not isinstance(seeds, list) or not 1 <= len(seeds) <= 16 or len(set(seeds)) != len(seeds):
        raise ValueError("need 1..16 distinct seeds")
    for seed in seeds: runner.scenario.integer(seed, 0, MASK)
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
    publish_json(output / "recipe.json", recipe)
    recipe_hash = runner.executable_digest(output / "recipe.json")
    results = []
    payload_bytes = 0
    try:
        for seed in seeds:
            for prefix in recipe["prefixes"]:
                for pages in PROFILES:
                    name = f"seed-{seed}-prefix-{prefix}-cache-{pages}"
                    value = scenario(seed, steps, prefix, pages)
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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--runner", type=Path, default=runner.ROOT / "target/debug/afsplus-scenario")
    parser.add_argument("--seeds", type=int, nargs="+", default=[1, 7, 42])
    parser.add_argument("--steps", type=int, default=96)
    parser.add_argument("--bundle-payload-mib", type=int, default=512)
    args = parser.parse_args()
    try:
        return 0 if qualify(args.output, args.runner, args.seeds, args.steps, args.bundle_payload_mib * 1024**2) else 1
    except (ValueError, OSError) as error:
        parser.exit(1, f"semantic qualification refused: {error}\n")


if __name__ == "__main__":
    raise SystemExit(main())
