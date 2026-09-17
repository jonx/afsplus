#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""Every exported boundary entry point is reachable from a target.

`api/afsplus_aros.h` is the C boundary of the AFS+ handler. An entry point
declared there and called from nothing a target runs is not a capability: no
program on AROS can use it, and a host test that calls it proves only that
the library works, not that the system does. Six of them were in that state
until they were found by hand.

This check reads the header and everything a target runs -- the packet
layer, the client library, the handler shell and the target tools -- and
fails when an export is reachable from none of them, unless the allow list
below names it with a reason.
"""

import re
import sys
from pathlib import Path

# An export that nothing on a target calls, with why that is right. Adding a
# name here is a decision to be argued in review, not a way to silence this.
ALLOWED = {
    # name: reason
}

DECLARATION = re.compile(r"\bafsplus_aros_[a-z0-9_]+\s*\(")
REFERENCE = re.compile(r"\bafsplus_aros_[a-z0-9_]+\b")


def main() -> int:
    root = Path(__file__).resolve().parent.parent
    header = root / "api" / "afsplus_aros.h"
    exported = {
        match.group(0)[:-1].strip()
        for match in DECLARATION.finditer(header.read_text())
    }
    if not exported:
        print("check-boundary-reachable: no entry points found in", header)
        return 2

    sources = [
        *(root / "native" / "aros").glob("*.c"),
        *(root / "native" / "aros" / "client").glob("*.c"),
        *(root / "native" / "aros" / "tools").glob("*.c"),
    ]
    reached: set[str] = set()
    for source in sources:
        reached.update(REFERENCE.findall(source.read_text()))

    unreachable = sorted(exported - reached - set(ALLOWED))
    stale = sorted(name for name in ALLOWED if name not in exported - reached)
    for name in unreachable:
        print(f"{name} is exported by api/afsplus_aros.h and called from no "
              "target path (packet layer, client, handler, target tools)")
    for name in stale:
        print(f"{name} is on the allow list of {Path(__file__).name} and does "
              "not need to be: remove it")
    if unreachable or stale:
        print(f"check-boundary-reachable result=FAIL exported={len(exported)} "
              f"unreachable={len(unreachable)} stale_allowed={len(stale)}")
        return 1
    print(f"check-boundary-reachable result=PASS exported={len(exported)} "
          f"allowed={len(ALLOWED)} sources={len(sources)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
