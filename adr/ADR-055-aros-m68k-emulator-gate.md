# ADR-055: Add a native AROS/m68k emulator gate before Amiga 500 hardware

Status: Accepted for pre-hardware qualification

## Context

AFS+ has already been qualified through Hosted MacAROS and native Apple-AArch64
QEMU. The classic target adds two independent risks: the AROS/m68k boot and DOS
ABI, and the availability of a trustworthy Rust or portable-C execution profile
on a 68000-class machine.

The official AROS m68k ISO is the system volume, not a self-contained Amiga boot
medium. The matching boot floppy starts `AROSBootstrap`, then assigns the volume
named exactly `AROS Live CD:` as `SYS:` and executes its `S:Startup-Sequence`.
Loading only `aros-rom.bin` and mounting the ISO does not exercise that protocol.

The available experimental Rust m68k target currently selects an M68020 CPU and
is therefore useful for an emulator bring-up, but cannot establish Amiga 500 or
plain-68000 compatibility. A stale custom toolchain path or a successful compile
alone is not runtime evidence.

## Decision

The port is qualified in four ordered stages:

1. Hosted MacAROS functional and crash-replay gates;
2. native Apple-AArch64 QEMU functional and crash-replay gates;
3. native AROS/m68k under an emulator; and
4. a physical Amiga 500.

Stage 3 begins with the machine-readable boot gate in
`tools/check-aros-m68k-boot-fsuae.sh`. It requires a matching official boot ADF
and system ISO, loads the ISO's ROM pair, leaves the boot ADF in `DF0:`, exposes
an extracted ISO as `AROS Live CD:`, and captures the guest verdict through a
separate `HOST:` volume. The gate records hashes and explicitly makes no A500,
68000, hardware, filesystem, or performance claim.

The subsequent m68k filesystem qualification has two explicit profiles:

- an M68020-or-newer emulator profile may first qualify the existing Rust
  reference engine and native handler semantics;
- the A500 gate must generate 68000-compatible code, avoid 68020-only CAS and
  other instructions, obey the classic resource profile, and run first on an
  A500-configured emulator before physical hardware.

The external-handler policy from ADR-050 remains authoritative. AFS+ must not
require a private AROS fork merely to install or mount the handler.

If qualification exposes an AROS defect, an AROS patch is appropriate only when
all of the following hold:

1. the defect is reproducible without depending on AFS+ internals;
2. the behavior violates a generic AROS API, ABI, DOS, device, or lifecycle
   contract;
3. a minimal regression test fails on the relevant upstream baseline; and
4. the proposed patch fixes that test without adding AFS+-specific policy.

Such a patch is developed and tested in the local AROS worktree and prepared for
upstream review. Until accepted, the AFS+ repository documents the required
baseline or patch. A driver-specific constraint stays in the external handler;
a real generic AROS bug must not be hidden behind an AFS+-only compatibility
shim.

## Consequences

- Emulator results cannot be presented as physical-A500 performance evidence.
- An M68020 Rust success does not close the 68000 code-generation gate.
- Boot-media and ROM hashes are part of every m68k evidence set.
- AROS modifications require a standalone reproducer and regression test, which
  makes the upstream/not-upstream boundary reviewable.
- The exact A500 memory, cache and workload budgets remain to be measured and
  accepted before stage 4.

ADR-056 completes the M68020-or-newer filesystem subprofile of stage 3,
including the Alpha-0 operation matrix and all six modeled intent-log cuts. The
plain-68000/A500-configured emulator subprofile remains open.
