# ADR-048: Split Hosted system-volume qualification into S1a and S1b

Status: Accepted for S1a; full S1 remains open

## Context

Calling a secondary-volume mount a system-volume result would overstate what has
been proved. Conversely, requiring the entire desktop and application suite in
the first pivot makes it difficult to distinguish filesystem/assign failures
from missing system payload.

AROS also stops reading the original Startup-Sequence as soon as `SYS:` is
replaced. A list of shell commands after `Assign SYS:` is therefore not a valid
bootstrap design: those lines are no longer guaranteed to execute.

## Decision

S1 is split into two cumulative gates:

- **S1a, core pivot:** mount a manifested AFS+ system subset, replace `SYS:`,
  `C:`, `L:`, `LIBS:`, `DEVS:` and `S:`, then execute commands and durable file
  I/O from that volume;
- **S1b, session pivot:** add the desktop, preferences and representative
  application manifest required by the original S1 acceptance definition.

`AFSPlusS1Pivot` is loaded by the bootstrap system before the replacement. It
pre-acquires every old/new lock, installs the six assigns without asking the old
Shell to read another script line, then synchronously executes
`S:S1-Sequence` from AFS+. `AFSPlusS1Probe`, which exists only in the AFS+
image, uses `SameLock` to prove all six logical paths name the expected AFS+
objects. It reads a provenance marker and durably creates `SYS:s1-runtime`.

`afsplus-populate` imports a deterministic host directory tree through the
portable core. The S1 builder emits content hashes, a clean pre-run checker and
a 64 MiB sparse image. The Hosted gate records the target outputs and requires
a clean post-run checker with no pending intent record.

## Bootstrap boundary

S1a explicitly retains these old-system dependencies:

- the running Shell and initial Startup-Sequence;
- `fdsk.device` and the already-loaded AFS+ handler;
- `AFSPlusS1Pivot`; and
- a `BOOTSYS:` emergency assign naming the distinct original volume.

No command in the post-pivot sequence uses `BOOTSYS:`. Removing it immediately
can block while AROS releases the old handler lock from inside a DosList
mutation; safe removal is a lifecycle hardening gate, not hidden inside S1a.

## Evidence

The Hosted S1a gate passed on 2026-08-30:

- 74 manifested payload files and 7 directories, 8,396,461 bytes;
- image generation 82 before target execution;
- `SYS/C/L/LIBS/DEVS/S` all verified on `AFSPLUS19`;
- `Version` and recursive `List` loaded and ran after the pivot;
- `SYS:s1-runtime` was durably created and copied to the host; and
- generation 84 after execution, checker-clean, zero pending log records.

The gate also exposed two non-S1a follow-ups. The DOSDriver file must retain the
logical name `AFSPLUS19`; installing it as `AFSPLUS19-S1` creates a different
device name. Separately, listing every assign crashes current AROS
`NameFromLock` on this mixed-handler table; the precise `SameLock` probe avoids
using that diagnostic as evidence, but the compatibility bug remains open.

## Consequences

S1a supports the narrow statement "AROS commands execute with their system
assigns on AFS+ after bootstrap." It does not yet satisfy the full statement
"a normal AROS session runs on AFS+"; that requires S1b.
