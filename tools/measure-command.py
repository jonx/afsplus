#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""Measure one host command with per-child CPU/RSS accounting; never use a shell."""
import argparse
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
import time


def measure(command, output):
    if sys.platform not in ("darwin", "linux") or not hasattr(os, "wait4"):
        raise ValueError("per-child RSS units are only supported on macOS and Linux")
    if not command:
        raise ValueError("a command is required")
    # Reserve the report before starting work; never replace an existing artifact.
    fd = os.open(output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    record = {
        "schema_version": 1,
        "command": command,
        "cwd": str(Path.cwd()),
        "system": platform.system(),
        "release": platform.release(),
        "machine": platform.machine(),
        "scope": "wait4 child accounting; not simultaneous process-tree peak memory",
        "environment_recorded": False,
    }
    start = time.monotonic_ns()
    process = None
    try:
        process = subprocess.Popen(command, stdin=subprocess.DEVNULL)
        _, status, usage = os.wait4(process.pid, 0)
        process.returncode = os.waitstatus_to_exitcode(status)
        record.update({
            "outcome": "exited",
            "returncode": process.returncode,
            "cpu_user_seconds": usage.ru_utime,
            "cpu_system_seconds": usage.ru_stime,
            "peak_rss_raw": usage.ru_maxrss,
            "peak_rss_raw_unit": "bytes" if sys.platform == "darwin" else "KiB",
            "peak_rss_bytes": usage.ru_maxrss * (1 if sys.platform == "darwin" else 1024),
        })
    except OSError as error:
        record.update({"outcome": "measurement_error", "error": str(error), "returncode": None})
    except BaseException as error:
        record.update({"outcome": "interrupted", "error": type(error).__name__, "returncode": None})
        raise
    finally:
        if process is not None and process.returncode is None:
            process.kill()
            process.wait()
        record["wall_seconds"] = (time.monotonic_ns() - start) / 1e9
        with os.fdopen(fd, "w") as stream:
            json.dump(record, stream, indent=2, allow_nan=False)
            stream.write("\n")
    code = record.get("returncode")
    return 127 if code is None else (code if code >= 0 else 128 - code)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    try:
        return measure(command, args.output)
    except (OSError, ValueError) as error:
        parser.error(str(error))


if __name__ == "__main__":
    raise SystemExit(main())
