# ADR-046: Hosted AROS same-image qualification

Status: Accepted

## Context

ADR-045 produced a link-complete AROS handler, but static link completion did
not prove the Resident startup contract. `genmodule` deliberately ignores the
`PROGRAM_ENTRIES` chain for handlers. The MacAROS Rust standard library link,
however, pulls per-opener `stdc`, `stdcio` and `posixc` bases plus INIT handlers
which expect those libraries to have been opened.

The first Hosted run reached `__posixc_startup` with a null `PosixCBase` before
the AFS+ handler body. After fixing that boundary, the next run exposed a second
host-specific assumption: an omitted Mountlist DMA mask inherited a classic
address window which cannot contain normal Hosted AArch64 allocations, while
`fdsk.device` itself has no DMA-address restriction.

A target-only create/read probe is still insufficient. The Alpha-0 claim
requires one image to cross both adapters and preserve the operations and bytes
created by each side.

## Decision

The generated AFS+ handler start is patched as a checked build input before it
is compiled. It opens the complete `LIBS` symbol set before calling INIT,
executes EXIT while those libraries remain open even when a later initializer
fails, and then closes every opened library. The patch has exact context and the
qualification script verifies the inserted calls, so a future incompatible
`genmodule` change fails the build rather than silently dropping the runtime
contract.

The reserved Hosted `fdsk.device` DOSDriver explicitly uses `Mask = 0`.
Hardware DOSDrivers continue to provide their real mask, which the native
trackdisk adapter honors with its bounded bounce buffer.

[`tools/check-hosted-aros-alpha0.sh`](../tools/check-hosted-aros-alpha0.sh) is the S0 acceptance gate. It refuses a
running Hosted instance or existing reserved artifacts and performs:

1. a fresh cross-qualified handler/package build;
2. target create, sparse write, fsync, truncate, rename and readback;
3. clean shutdown and strict host checker;
4. a real macFUSE mount of that same image, target-file readback and the same
   host operation slice;
5. clean host unmount and strict checker;
6. a second Hosted boot which reads both cross-created files; and
7. final checker, hashes, evidence capture and removal of installed artifacts.

macOS AppleDouble files are ordinary valid files while AFS+ lacks xattrs, but
the host probe removes its own two possible sidecars so they cannot pollute the
fixture or later benchmark counts.

## Consequences

Hosted S0 is now demonstrated bidirectionally: the target-created file contains
`hello`, the host-created file contains `host`, and the final returned image is
clean. The checked run advanced from generation 6 after the first target phase
to generation 21 after the host phase; the return-read phase made no changes.

ADR-052 requalification additionally performs a case-only target rename and a
folded reopen. The returned identification-v3 image reports Unicode 16.0.0
`unicode-nfc-casefold`; it is checker-clean at generation 22 with three objects
and no pending intent record.

This proves native handler execution and same-image interoperability on Hosted
MacAROS. It does not prove target power-cut replay, an AFS+ `SYS:` pivot,
boot-volume selection, native Apple Silicon execution, m68k runtime behavior or
physical Amiga performance. Those claims retain separate gates in
[`testing/aros-system-volume-qualification.md`](../testing/aros-system-volume-qualification.md).
