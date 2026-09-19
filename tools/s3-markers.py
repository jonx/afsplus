#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""The host side of S3: the cut schedule, the marker bytes and the verdict.

`tools/check-hosted-aros-s3.sh` boots AROS from an AFS+ partition, cuts it off
and boots it again. This program decides, off the target:

  schedule  which moment each round is cut at, and how long after the phase
            line the cut falls; from the round number and a fixed seed alone,
            so a run can be repeated exactly and the schedule is printed
            before anything boots.
  marker    the bytes a marker file must hold for a round and seed: the same
            header, payload and trailer the guest program builds
            (native/aros/tools/afsplus_s3_workload.c).
  record    one round's verdict from the markers the guest copied out: a
            marker claimed flushed before the cut must be there and identical;
            one only written may be there or not, but never half written.
  summary   the per-round table, the leak clause on boot time, and the exit
            status of the whole run.

Exit 0 when everything holds, 1 when a check fails, 2 on a usage error.
"""
import argparse
import json
import os
import sys

MARKER_BYTES = 4096
MOMENTS = ("startup", "writes", "after-flush", "idle", "clean")
# How long after the phase line the cut falls, per moment, in milliseconds.
JITTER = {
    "startup": (300, 3000),
    "writes": (500, 5000),
    "after-flush": (0, 400),
    "idle": (1000, 6000),
    "clean": (0, 0),
}
EXIT_OK = 0
EXIT_FAIL = 1
EXIT_USAGE = 2


def xorshift(state):
    """The generator of the guest program, 32 bits wide."""
    state ^= (state << 13) & 0xFFFFFFFF
    state ^= state >> 17
    state ^= (state << 5) & 0xFFFFFFFF
    return state & 0xFFFFFFFF


def stream_state(seed, index):
    state = (seed ^ (index * 2654435761) ^ 0x9E3779B9) & 0xFFFFFFFF
    return state if state != 0 else 1


class Random:
    def __init__(self, state):
        self.state = state

    def next(self):
        self.state = xorshift(self.state)
        return self.state

    def below(self, bound):
        return self.next() % bound if bound > 0 else 0


def marker_bytes(seed, round_number):
    """The whole content of a marker file: header, payload, trailer."""
    random = Random(stream_state(seed, round_number))
    payload = bytearray(MARKER_BYTES)
    digest = 2166136261
    for at in range(MARKER_BYTES):
        byte = random.next() & 0xFF
        payload[at] = byte
        digest = ((digest ^ byte) * 16777619) & 0xFFFFFFFF
    header = f"AFSPLUS-S3 marker {round_number} hash {digest:08x}\n"
    trailer = f"AFSPLUS-S3 end {round_number}\n"
    return header.encode("ascii") + bytes(payload) + trailer.encode("ascii")


def permutation(seed, block):
    """The five moments in an order that depends on the seed and the block."""
    random = Random(stream_state(seed, 1000 + block))
    order = list(MOMENTS)
    for at in range(len(order) - 1, 0, -1):
        other = random.below(at + 1)
        order[at], order[other] = order[other], order[at]
    return order


def schedule(seed, rounds, pinned=None):
    """One (round, moment, jitter_ms) per round.

    Every block of five rounds carries each moment exactly once, so a six-
    round development run covers all five and a cut moment is never starved.
    """
    plan = []
    for round_number in range(1, rounds + 1):
        block, index = divmod(round_number - 1, len(MOMENTS))
        moment = pinned if pinned else permutation(seed, block)[index]
        low, high = JITTER[moment]
        random = Random(stream_state(seed, round_number))
        jitter = low if high <= low else low + random.below(high - low + 1)
        plan.append((round_number, moment, jitter))
    return plan


def classify(path, expected):
    """What a marker file on the host is: whole, torn, empty or absent."""
    if not os.path.exists(path):
        return "absent"
    with open(path, "rb") as handle:
        found = handle.read()
    if len(found) == 0:
        return "empty"
    if found == expected:
        return "whole"
    return "torn"


def load_state(path):
    if os.path.exists(path):
        with open(path, "r", encoding="utf-8") as handle:
            return json.load(handle)
    return {"seed": 0, "rounds": [], "flush_claimed": [], "soft_claimed": [],
            "soft_durable": [], "violations": []}


def save_state(path, state):
    with open(path, "w", encoding="utf-8") as handle:
        json.dump(state, handle, indent=1, sort_keys=True)
        handle.write("\n")


def judge_preference(directory, round_number):
    """The preference saved under ENVARC: is whole or it is not there."""
    path = os.path.join(directory, "envarc-prefs")
    if not os.path.exists(path):
        return None, []
    with open(path, "rb") as handle:
        found = handle.read()
    if found == b"":
        return "empty", []
    text = found.decode("ascii", "replace").strip()
    parts = text.split()
    if (len(parts) != 3 or parts[0] != "AFSPlusS3" or parts[1] != "round"
            or not parts[2].isdigit() or int(parts[2]) > round_number
            or not found.endswith(b"\n")):
        return "torn", [f"the ENVARC: preference is not a whole save: {text!r}"]
    return "whole", []


def record(arguments):
    state = load_state(arguments.state)
    state["seed"] = arguments.seed
    for claimed, key in ((arguments.claimed_flush, "flush_claimed"),
                         (arguments.claimed_soft, "soft_claimed")):
        if claimed and arguments.round not in state[key]:
            state[key].append(arguments.round)
    violations = []
    if arguments.boot_ok != "yes":
        violations.append(f"round {arguments.round}: the boot after the cut "
                          "did not reach the Startup-Sequence on AFS+")
    if arguments.clean != "yes":
        violations.append(f"round {arguments.round}: the partition did not "
                          "check clean")
    if arguments.pending != 0:
        violations.append(f"round {arguments.round}: "
                          f"log_records_pending {arguments.pending} after the "
                          "mount that followed the cut")
    kept = lost = torn = soft_absent = 0
    for claimed_round in sorted(state["flush_claimed"]):
        path = os.path.join(arguments.markers, f"m{claimed_round}.flush")
        what = classify(path, marker_bytes(arguments.seed, claimed_round))
        if what == "whole":
            kept += 1
        elif what == "torn":
            torn += 1
            violations.append(f"round {arguments.round}: the flushed marker "
                              f"of round {claimed_round} is torn")
        else:
            lost += 1
            violations.append(f"round {arguments.round}: the flushed marker "
                              f"of round {claimed_round} is {what}")
    for claimed_round in sorted(state["soft_claimed"]):
        path = os.path.join(arguments.markers, f"m{claimed_round}.soft")
        what = classify(path, marker_bytes(arguments.seed, claimed_round))
        if what == "whole":
            kept += 1
            if claimed_round not in state["soft_durable"]:
                state["soft_durable"].append(claimed_round)
        elif what == "torn":
            torn += 1
            violations.append(f"round {arguments.round}: the soft marker of "
                              f"round {claimed_round} is torn")
        elif claimed_round in state["soft_durable"]:
            lost += 1
            violations.append(f"round {arguments.round}: the soft marker of "
                              f"round {claimed_round} was there after an "
                              f"earlier cut and is now {what}")
        else:
            soft_absent += 1
    preference, complaints = judge_preference(arguments.markers,
                                              arguments.round)
    violations.extend(complaints)
    state["rounds"].append({
        "round": arguments.round,
        "moment": arguments.moment,
        "jitter_ms": arguments.jitter_ms,
        "stage": arguments.stage,
        "boot_ok": arguments.boot_ok == "yes",
        "clean": arguments.clean == "yes",
        "pending": arguments.pending,
        "boot_seconds": round(arguments.boot_seconds, 1),
        "kept": kept,
        "lost": lost,
        "torn": torn,
        "soft_absent": soft_absent,
        "preference": preference,
    })
    state["violations"].extend(violations)
    save_state(arguments.state, state)
    for line in violations:
        print(f"[hosted-s3] {line}")
    return EXIT_OK


HEADINGS = ("round", "cut moment", "at", "stage", "boot", "clean", "pend",
            "kept", "lost", "torn", "soft-", "boot s")


def table(state):
    rows = [HEADINGS]
    for entry in state["rounds"]:
        rows.append((
            str(entry["round"]),
            entry["moment"],
            f"{entry['jitter_ms']}ms",
            entry["stage"],
            "ok" if entry["boot_ok"] else "FAIL",
            "ok" if entry["clean"] else "FAIL",
            str(entry["pending"]),
            str(entry["kept"]),
            str(entry["lost"]),
            str(entry["torn"]),
            str(entry["soft_absent"]),
            f"{entry['boot_seconds']:.1f}",
        ))
    widths = [max(len(row[at]) for row in rows) for at in range(len(HEADINGS))]
    lines = []
    for index, row in enumerate(rows):
        lines.append("  ".join(cell.ljust(widths[at])
                               for at, cell in enumerate(row)).rstrip())
        if index == 0:
            lines.append("  ".join("-" * width for width in widths))
    return lines


def leak_clause(state):
    """Boot time must not grow steadily over the run."""
    times = [entry["boot_seconds"] for entry in state["rounds"]
             if entry["boot_ok"]]
    if len(times) < 6:
        return None, []
    first = sum(times[:3]) / 3.0
    last = times[-3:]
    note = (f"boot time: first three {first:.1f} s mean, "
            f"last three {last[0]:.1f}/{last[1]:.1f}/{last[2]:.1f} s")
    if first > 0 and all(one > 2.0 * first for one in last):
        return note, [f"boot time grows: each of the last three rounds is "
                      f"more than twice the mean of the first three ({note})"]
    return note, []


def summary(arguments):
    state = load_state(arguments.state)
    lines = table(state)
    note, complaints = leak_clause(state)
    violations = list(state["violations"]) + complaints
    counted = {}
    for entry in state["rounds"]:
        counted[entry["moment"]] = counted.get(entry["moment"], 0) + 1
    lines.append("")
    lines.append("cuts per moment: " + ", ".join(
        f"{moment} {counted.get(moment, 0)}" for moment in MOMENTS))
    lines.append("markers kept in the last round: " + (
        str(state["rounds"][-1]["kept"]) if state["rounds"] else "0"))
    lines.append("torn markers: " + str(
        sum(entry["torn"] for entry in state["rounds"])))
    if note:
        lines.append(note)
    for line in violations:
        lines.append("FAIL " + line)
    text = "\n".join(lines) + "\n"
    if arguments.output:
        with open(arguments.output, "w", encoding="utf-8") as handle:
            handle.write(text)
    sys.stdout.write(text)
    return EXIT_FAIL if violations else EXIT_OK


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)

    one = commands.add_parser("schedule")
    one.add_argument("--seed", type=int, required=True)
    one.add_argument("--rounds", type=int, required=True)
    one.add_argument("--moment", choices=MOMENTS,
                     help="pin every round to one moment (negative controls)")

    two = commands.add_parser("marker")
    two.add_argument("--seed", type=int, required=True)
    two.add_argument("--round", type=int, required=True)
    two.add_argument("--output", required=True)

    three = commands.add_parser("record")
    three.add_argument("--state", required=True)
    three.add_argument("--markers", required=True)
    three.add_argument("--seed", type=int, required=True)
    three.add_argument("--round", type=int, required=True)
    three.add_argument("--moment", required=True)
    three.add_argument("--jitter-ms", type=int, required=True)
    three.add_argument("--stage", required=True)
    three.add_argument("--boot-ok", choices=("yes", "no"), required=True)
    three.add_argument("--clean", choices=("yes", "no"), required=True)
    three.add_argument("--pending", type=int, required=True)
    three.add_argument("--boot-seconds", type=float, required=True)
    three.add_argument("--claimed-flush", action="store_true")
    three.add_argument("--claimed-soft", action="store_true")

    four = commands.add_parser("summary")
    four.add_argument("--state", required=True)
    four.add_argument("--output")

    arguments = parser.parse_args(argv)
    if arguments.command == "schedule":
        for round_number, moment, jitter in schedule(
                arguments.seed, arguments.rounds, arguments.moment):
            print(f"{round_number} {moment} {jitter}")
        return EXIT_OK
    if arguments.command == "marker":
        with open(arguments.output, "wb") as handle:
            handle.write(marker_bytes(arguments.seed, arguments.round))
        return EXIT_OK
    if arguments.command == "record":
        return record(arguments)
    return summary(arguments)


if __name__ == "__main__":
    sys.exit(main())
