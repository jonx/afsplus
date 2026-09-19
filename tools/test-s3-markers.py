#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""Made-up markers for the host side of S3, before any of it meets AROS.

Whole, torn, empty and absent files are written by hand into a temporary
directory and put through the same verdict the real run uses, so a wrong
verdict shows here instead of in a twenty-minute boot loop.
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

TOOL = Path(__file__).with_name("s3-markers.py")
SEED = 4242


def run(*arguments):
    return subprocess.run([sys.executable, str(TOOL), *arguments],
                          capture_output=True, text=True)


def marker(directory, name, content):
    path = os.path.join(directory, name)
    with open(path, "wb") as handle:
        handle.write(content)
    return path


class ScheduleTests(unittest.TestCase):
    def test_the_same_seed_gives_the_same_schedule(self):
        first = run("schedule", "--seed", str(SEED), "--rounds", "24").stdout
        second = run("schedule", "--seed", str(SEED), "--rounds", "24").stdout
        self.assertEqual(first, second)
        self.assertNotEqual(
            first, run("schedule", "--seed", "7", "--rounds", "24").stdout)

    def test_every_moment_appears_in_the_first_five_rounds(self):
        moments = [line.split()[1] for line in
                   run("schedule", "--seed", str(SEED),
                       "--rounds", "6").stdout.splitlines()]
        self.assertEqual(sorted(moments[:5]),
                         sorted(["startup", "writes", "after-flush", "idle",
                                 "clean"]))
        self.assertEqual(len(moments), 6)

    def test_a_pinned_moment_holds_for_every_round(self):
        lines = run("schedule", "--seed", str(SEED), "--rounds", "4",
                    "--moment", "writes").stdout.splitlines()
        self.assertEqual([line.split()[1] for line in lines], ["writes"] * 4)
        self.assertTrue(all(500 <= int(line.split()[2]) <= 5000
                            for line in lines))

    def test_the_cut_of_a_clean_round_waits_for_nothing(self):
        lines = run("schedule", "--seed", str(SEED), "--rounds", "5").stdout
        for line in lines.splitlines():
            if line.split()[1] == "clean":
                self.assertEqual(line.split()[2], "0")


class VerdictTests(unittest.TestCase):
    def judge(self, directory, state, round_number, **keywords):
        arguments = ["record", "--state", state, "--markers", directory,
                     "--seed", str(SEED), "--round", str(round_number),
                     "--moment", keywords.get("moment", "writes"),
                     "--jitter-ms", "700", "--stage", "workload",
                     "--boot-ok", keywords.get("boot_ok", "yes"),
                     "--clean", keywords.get("clean", "yes"),
                     "--pending", str(keywords.get("pending", 0)),
                     "--boot-seconds", str(keywords.get("boot_seconds", 20.0))]
        if keywords.get("claimed_flush", True):
            arguments.append("--claimed-flush")
        if keywords.get("claimed_soft", True):
            arguments.append("--claimed-soft")
        return run(*arguments)

    def whole(self, round_number):
        with tempfile.NamedTemporaryFile(delete=False) as handle:
            path = handle.name
        run("marker", "--seed", str(SEED), "--round", str(round_number),
            "--output", path)
        with open(path, "rb") as handle:
            content = handle.read()
        os.unlink(path)
        return content

    def test_a_whole_flushed_marker_is_kept(self):
        with tempfile.TemporaryDirectory() as directory:
            state = os.path.join(directory, "state.json")
            markers = os.path.join(directory, "markers")
            os.mkdir(markers)
            marker(markers, "m1.flush", self.whole(1))
            marker(markers, "m1.soft", self.whole(1))
            self.judge(markers, state, 1)
            with open(state, encoding="utf-8") as handle:
                recorded = json.load(handle)
            self.assertEqual(recorded["violations"], [])
            self.assertEqual(recorded["rounds"][0]["kept"], 2)
            self.assertEqual(run("summary", "--state", state).returncode, 0)

    def test_a_lost_flushed_marker_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            state = os.path.join(directory, "state.json")
            markers = os.path.join(directory, "markers")
            os.mkdir(markers)
            marker(markers, "m1.soft", self.whole(1))
            result = self.judge(markers, state, 1)
            self.assertIn("is absent", result.stdout)
            summarised = run("summary", "--state", state)
            self.assertEqual(summarised.returncode, 1)
            self.assertIn("FAIL", summarised.stdout)

    def test_a_torn_marker_fails_whether_short_or_altered(self):
        for name, content in (("short", self.whole(1)[:2048]),
                              ("altered", self.whole(1)[:-1] + b"X")):
            with self.subTest(name=name):
                with tempfile.TemporaryDirectory() as directory:
                    state = os.path.join(directory, "state.json")
                    markers = os.path.join(directory, "markers")
                    os.mkdir(markers)
                    marker(markers, "m1.flush", content)
                    marker(markers, "m1.soft", content)
                    self.judge(markers, state, 1)
                    with open(state, encoding="utf-8") as handle:
                        recorded = json.load(handle)
                    self.assertEqual(recorded["rounds"][0]["torn"], 2)
                    self.assertEqual(
                        run("summary", "--state", state).returncode, 1)

    def test_a_soft_marker_may_be_absent_or_empty_but_not_after_it_was_there(self):
        with tempfile.TemporaryDirectory() as directory:
            state = os.path.join(directory, "state.json")
            markers = os.path.join(directory, "markers")
            os.mkdir(markers)
            marker(markers, "m1.flush", self.whole(1))
            marker(markers, "m1.soft", b"")
            self.judge(markers, state, 1)
            with open(state, encoding="utf-8") as handle:
                recorded = json.load(handle)
            self.assertEqual(recorded["rounds"][0]["soft_absent"], 1)
            self.assertEqual(recorded["violations"], [])
            # It arrives in the next round and must then stay.
            marker(markers, "m1.soft", self.whole(1))
            marker(markers, "m2.flush", self.whole(2))
            marker(markers, "m2.soft", self.whole(2))
            self.judge(markers, state, 2)
            os.unlink(os.path.join(markers, "m1.soft"))
            self.judge(markers, state, 3, claimed_flush=False,
                       claimed_soft=False)
            summarised = run("summary", "--state", state)
            self.assertEqual(summarised.returncode, 1)
            self.assertIn("was there after an earlier cut", summarised.stdout)

    def test_an_unclaimed_marker_is_not_judged(self):
        with tempfile.TemporaryDirectory() as directory:
            state = os.path.join(directory, "state.json")
            markers = os.path.join(directory, "markers")
            os.mkdir(markers)
            # The cut fell inside the write: a prefix on disk, never claimed.
            marker(markers, "m1.flush", self.whole(1)[:1000])
            self.judge(markers, state, 1, claimed_flush=False,
                       claimed_soft=False)
            self.assertEqual(run("summary", "--state", state).returncode, 0)

    def test_a_failed_boot_unclean_image_or_pending_log_fails(self):
        for name, keywords in (("boot", {"boot_ok": "no"}),
                               ("clean", {"clean": "no"}),
                               ("pending", {"pending": 3})):
            with self.subTest(name=name):
                with tempfile.TemporaryDirectory() as directory:
                    state = os.path.join(directory, "state.json")
                    markers = os.path.join(directory, "markers")
                    os.mkdir(markers)
                    marker(markers, "m1.flush", self.whole(1))
                    marker(markers, "m1.soft", self.whole(1))
                    self.judge(markers, state, 1, **keywords)
                    self.assertEqual(
                        run("summary", "--state", state).returncode, 1)

    def test_a_half_written_preference_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            state = os.path.join(directory, "state.json")
            markers = os.path.join(directory, "markers")
            os.mkdir(markers)
            marker(markers, "m1.flush", self.whole(1))
            marker(markers, "m1.soft", self.whole(1))
            marker(markers, "envarc-prefs", b"AFSPlusS3 rou")
            self.judge(markers, state, 1)
            self.assertEqual(run("summary", "--state", state).returncode, 1)

    def test_boot_time_that_doubles_at_the_end_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            state = os.path.join(directory, "state.json")
            markers = os.path.join(directory, "markers")
            os.mkdir(markers)
            for round_number in range(1, 7):
                marker(markers, f"m{round_number}.flush",
                       self.whole(round_number))
                marker(markers, f"m{round_number}.soft",
                       self.whole(round_number))
                self.judge(markers, state, round_number,
                           boot_seconds=10.0 if round_number <= 3 else 30.0)
            summarised = run("summary", "--state", state)
            self.assertEqual(summarised.returncode, 1)
            self.assertIn("boot time grows", summarised.stdout)

    def test_steady_boot_time_passes(self):
        with tempfile.TemporaryDirectory() as directory:
            state = os.path.join(directory, "state.json")
            markers = os.path.join(directory, "markers")
            os.mkdir(markers)
            for round_number in range(1, 7):
                marker(markers, f"m{round_number}.flush",
                       self.whole(round_number))
                marker(markers, f"m{round_number}.soft",
                       self.whole(round_number))
                self.judge(markers, state, round_number,
                           boot_seconds=20.0 + round_number)
            self.assertEqual(run("summary", "--state", state).returncode, 0)


if __name__ == "__main__":
    unittest.main()
