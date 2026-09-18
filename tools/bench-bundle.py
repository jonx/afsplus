#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""The result bundle of tools/bench-hosted-aros.sh: results.json in the
format of testing/benchmark-contract.md section 7, built from what the run
left in its result directory. Every measurement is copied from a runner
line; a line that is missing or malformed fails the bundle."""

import argparse
import json
import platform
import re
import subprocess
import sys
from pathlib import Path

PHASES = ("create", "list", "read", "rename", "delete")
PHASE = re.compile(r"^\[AFSPLUS-BENCH\] phase (\w+) ops (\d+) us (\d+)$")
WORKLOAD = re.compile(
    r"^\[AFSPLUS-BENCH\] workload (\w+) seed (\d+) trees (\d+) drawers (\d+)"
    r" files (\d+) bytes (\d+)$"
)
CLOCK = re.compile(r"^\[AFSPLUS-BENCH\] clock step_us (\d+)$")
COUNTERS = re.compile(r"^\[AFSPLUS-BENCH\] counters (before|after) (.*)$")


def fail(message):
    print(f"bench-bundle: {message}", file=sys.stderr)
    sys.exit(1)


def mountlist(path):
    values = {}
    for line in Path(path).read_text().splitlines():
        if "=" in line:
            key, value = line.split("=", 1)
            values[key.strip()] = value.strip()
    return values


def run(path):
    lines = Path(path).read_text().splitlines()
    workload = None
    clock_step = None
    phases = {}
    counters = {}
    for line in lines:
        if match := CLOCK.match(line):
            clock_step = int(match[1])
        elif match := WORKLOAD.match(line):
            workload = {
                "name": match[1],
                "seed": int(match[2]),
                "trees": int(match[3]),
                "drawers": int(match[4]),
                "files": int(match[5]),
                "payload_bytes": int(match[6]),
            }
        elif match := PHASE.match(line):
            phases[match[1]] = {"operations": int(match[2]), "elapsed_us": int(match[3])}
        elif match := COUNTERS.match(line):
            words = match[2].split()
            if len(words) % 2:
                fail(f"{path}: odd counter line")
            counters[match[1]] = {
                words[index]: int(words[index + 1]) for index in range(0, len(words), 2)
            }
    if workload is None or clock_step is None:
        fail(f"{path}: no workload or clock line")
    if sorted(phases) != sorted(PHASES):
        fail(f"{path}: phases {sorted(phases)}")
    measured = {
        "workload": workload,
        "clock_step_us": clock_step,
        "phases": [dict(name=p, **phases[p]) for p in PHASES],
    }
    measured["elapsed_us"] = sum(phases[p]["elapsed_us"] for p in PHASES)
    if counters:
        before, after = counters.get("before"), counters.get("after")
        if before is None or after is None:
            fail(f"{path}: counters before and after are both required")
        measured["counters_before"] = before
        measured["counters_after"] = after
        measured["counters_delta"] = {
            key: after[key] - before[key]
            for key in after
            if key not in ("heap", "heap_peak", "cache_blocks")
        }
        written = measured["counters_delta"]["written_bytes"]
        measured["write_amplification"] = round(written / workload["payload_bytes"], 3)
    return measured


def git(repo, *arguments):
    return subprocess.run(
        ["git", "-C", repo, *arguments], check=True, capture_output=True, text=True
    ).stdout.strip()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--result", required=True)
    parser.add_argument("--seed", type=int, required=True)
    parser.add_argument("--repo", required=True)
    parser.add_argument("--mountlist", required=True)
    parser.add_argument("--baseline-mountlist", required=True)
    parser.add_argument("--added-buffers", type=int, default=0)
    arguments = parser.parse_args()
    result = Path(arguments.result)

    afsplus = run(result / "afsplus.out")
    baseline = run(result / "baseline.out")
    for measured in (afsplus, baseline):
        if measured["workload"]["seed"] != arguments.seed:
            fail("a run used another seed")
    if afsplus["workload"] != baseline["workload"]:
        fail("the two runs did not run the same workload")
    if "counters_after" not in afsplus:
        fail("the AFS+ run carries no handler counters")

    check_before = json.loads((result / "check-before.json").read_text())
    check_after = json.loads((result / "check-after.json").read_text())
    images = dict(
        reversed(line.split(maxsplit=1)) for line in
        (result / "images-before.sha256").read_text().splitlines()
    )
    bundle = {
        "schema": "afsplus-bench-hosted-aros",
        "version": 1,
        "git_commit": git(arguments.repo, "rev-parse", "HEAD"),
        "git_dirty": bool(git(arguments.repo, "status", "--porcelain", "--untracked-files=no")),
        "platform": {
            "target": "Hosted MacAROS darwin-aarch64",
            "host_os": platform.platform(),
            "host_machine": platform.machine(),
            "note": "Hosted gives software cost; storage is a host file behind fdsk.device",
        },
        "build_profile": (result / "build-profile.txt").read_text().splitlines(),
        "package_manifest": (result / "package-SHA256SUMS").read_text().splitlines(),
        "images_before_sha256": images,
        "configuration": {
            "workload": afsplus["workload"],
            "afsplus_mount": mountlist(arguments.mountlist),
            "baseline_mount": mountlist(arguments.baseline_mountlist),
            "afsplus_added_buffers": arguments.added_buffers,
            "cache": "AFS+ read cache: the DOSDriver Buffers plus AddBuffers, in blocks;"
            " the counters give the size in force",
        },
        "verification": {
            "before": {"package_manifest": "verified", "afsplus_image_clean": check_before.get("clean")},
            "after": {
                "afsplus_image_clean": check_after.get("clean"),
                "baseline": "no host checker for the Fast File System; every file read back byte for byte",
            },
        },
        "measurements": {"afsplus": afsplus, "baseline_ffs": baseline},
        "not_measured": [
            "CPU user and system time: AROS keeps no per-task CPU accounting",
            "baseline device I/O: the Fast File System reports no counters",
        ],
    }
    json.dump(bundle, sys.stdout, indent=2, sort_keys=True)
    print()


if __name__ == "__main__":
    main()
