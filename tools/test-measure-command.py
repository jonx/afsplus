#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""Private temporary-fixture tests for per-command accounting."""
import json
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import unittest

TOOL = Path(__file__).with_name("measure-command.py")


class MeasurementTests(unittest.TestCase):
    def run_command(self, directory, *command):
        output = Path(directory) / "measurement.json"
        result = subprocess.run([sys.executable, str(TOOL), "--output", str(output), "--", *command],
                                capture_output=True, text=True)
        return result, output

    def test_success_cpu_memory_and_literal_arguments(self):
        with tempfile.TemporaryDirectory() as directory:
            literal = "$(touch should-not-exist); `false`"
            result, path = self.run_command(directory, sys.executable, "-c",
                "import sys; a=bytearray(8*1024*1024); sum(range(200000)); print(sys.argv[1])", literal)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout.strip(), literal)
            report = json.loads(path.read_text())
            self.assertEqual(report["returncode"], 0)
            self.assertGreater(report["cpu_user_seconds"], 0)
            self.assertGreater(report["peak_rss_bytes"], 0)
            self.assertGreater(report["wall_seconds"], 0)
            self.assertEqual(report["peak_rss_bytes"], report["peak_rss_raw"] *
                             (1 if sys.platform == "darwin" else 1024))
            self.assertFalse(report["environment_recorded"])

    def test_failure_and_signal_are_preserved(self):
        for script, expected, raw in [("raise SystemExit(7)", 7, 7),
                ("import os,signal; os.kill(os.getpid(),signal.SIGTERM)", 128 + signal.SIGTERM, -signal.SIGTERM)]:
            with tempfile.TemporaryDirectory() as directory:
                result, path = self.run_command(directory, sys.executable, "-c", script)
                self.assertEqual(result.returncode, expected)
                self.assertEqual(json.loads(path.read_text())["returncode"], raw)

    def test_existing_report_prevents_child_execution(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "measurement.json"
            path.write_text("keep")
            result, _ = self.run_command(directory, sys.executable, "-c", "print('CHILD RAN')")
            self.assertNotEqual(result.returncode, 0)
            self.assertNotIn("CHILD RAN", result.stdout)
            self.assertEqual(path.read_text(), "keep")

    def test_spawn_error_is_not_a_successful_measurement(self):
        with tempfile.TemporaryDirectory() as directory:
            result, path = self.run_command(directory, str(Path(directory) / "missing"))
            self.assertEqual(result.returncode, 127)
            report = json.loads(path.read_text())
            self.assertEqual(report["outcome"], "measurement_error")
            self.assertIsNone(report["returncode"])
            self.assertNotIn("peak_rss_bytes", report)


if __name__ == "__main__":
    unittest.main()
