# ADR-053: Qualify native MacAROS with a retained-image block transport

Status: Accepted for pre-hardware QEMU qualification

## Context

The off-tree handler linked against the `apple-aarch64` SDK, but a link result
did not prove that AROS could load it, mount an AFS+ image, run DOS operations
and dismantle the complete stack. The current MacAROS bootstrap exposes a
verified FAT image retained in RAM. It does not yet expose a persistent native
disk suitable for this qualification.

Putting an AFS+-specific driver in the AROS core would make the test depend on
upstream inclusion and would confuse a temporary QEMU transport with the final
hardware path.

## Decision

Keep the pre-hardware transport in the AFS+ repository as the external
`afsram.device`. It obtains the retained boot image through the public
`KrnGetSystemAttr` interface and exposes a bounded, 512-byte-aligned writable
trackdisk view over the AFS+ payload. Reads and writes copy directly to retained
RAM. `CMD_UPDATE` is an ordering point for the filesystem stack but cannot make
RAM survive a reset, so this transport makes no persistence claim.

Composite descriptor version 2 has this deterministic layout:

1. an exact FAT12 bootstrap image;
2. a 4-KiB descriptor with magic, version, flags and exact sizes;
3. the raw AROS handler module, followed by zero padding to 4 KiB; and
4. the AFS+ payload occupying the exact remaining tail.

The shared C parser rejects unknown flags or versions, nonzero reserved bytes,
overflow, misalignment, nonzero padding and any trailing bytes. The small FAT
contains only the boot sentinels, operation probe and `afsram.device`. The QEMU
harness loads the raw handler from the retained image through the public
`InternalLoadSeg` callback API. This raw loading path is test-bootstrap policy;
normal installations continue to use an `L:` handler and DOSDriver.

The gate snapshots the MacAROS and AROS worktree status before building and
requires it to be unchanged afterward. Its evidence report hashes the bundle,
composite, selected probe, device, raw handler, kernel resource and stage 2.

## Evidence

On 2026-08-30 the `apple-aarch64` QEMU gate loaded `afsram.device` and the
complete external AFS+ handler, mounted the same 64-MiB image and passed native
DOS create, read, write, sparse write, two flushes, truncate, rename,
case-insensitive lookup, case-only rename and reopen/readback. It then inhibited
and stopped the handler, waited for its task to disappear, freed the DOS mount
node, unloaded the handler and device, and exited QEMU without a fault.

The final cleanup investigation also established that `MakeDosNode()` owns one
packed allocation containing its startup message, environment and BSTRs. The
probe now frees only its separately allocated handler BSTR before freeing that
packed allocation once. The observed hang was therefore a probe bug, not an
AROS `UnLoadSeg` or page-protection defect; no AROS patch was warranted.

## Consequences

Native MacAROS execution is now proven at the pre-hardware QEMU stage without
adding AFS+ to the AROS source tree. The transport and raw loader remain bounded
test infrastructure and are not a production storage driver.

This gate does not yet prove reset durability, extraction and strict checking
of the mutated RAM image, native crash replay, Apple-hardware execution or m68k
performance. Those claims require separate evidence rather than
reinterpretation of this PASS.
