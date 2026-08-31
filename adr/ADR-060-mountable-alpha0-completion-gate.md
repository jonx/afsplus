# ADR-060: Close Mountable Alpha-0 with one composite gate

Status: Accepted

## Context

The Mountable Alpha-0 objective spans layers that can each pass independently:
intent-log recovery, the portable VFS API, FUSE protocol translation, a real
host mount, Hosted AROS packet behavior and native MacAROS execution. Separate
green runs did not by themselves prove that the exact objective remained true
as a whole.

A completion claim requires direct evidence that one image crosses the host and
AROS boundary with the promised operation set, that both platform paths replay
the same deterministic crash states, and that every final image is structurally
clean with no pending intent record.

## Decision

[`tools/check-mountable-alpha0.sh`](../tools/check-mountable-alpha0.sh) is the composite completion gate. It runs:

1. the portable VFS API, FUSE protocol, intent-log and AROS adapter tests;
2. Hosted AROS → real macFUSE → Hosted AROS on one image;
3. all six Hosted intent-log recovery fixtures;
4. native Apple-AArch64 MacAROS Alpha-0 under QEMU; and
5. the same six recovery fixtures under native MacAROS QEMU.

The gate independently verifies every child result, exact cross-created file
contents, handler shutdown/restart/dismount statuses, checker cleanliness, zero
pending log records, four old plus two new replay outcomes, and the absence of
a guest failure requester. It emits a requirement-by-requirement TSV report and
a checksum manifest over the retained evidence.

The work directory is deliberately rooted at a short `/tmp` path because the
macOS Unix-domain QMP socket limit applies to nested native runs. A completed
set of child gates can be copied and reverified with
`AFSPLUS_MOUNTABLE_ALPHA0_REUSE_RESULT`; reuse never moves or overwrites the
source evidence.

Native input integrity is based on byte fingerprints of the tools, harness,
EFI, stage 2, module generator and SDK configuration actually consumed by the
gate. This permits unrelated source-tree edits by a parallel agent while still
rejecting a change to a runtime/build input during a case.

## Evidence

The accepted 2026-08-31 run produced `format=afsplus-mountable-alpha0-v1` and
`result=PASS` with:

- all portable VFS, FUSE protocol, intent-log and AROS adapter tests passing;
- one real macFUSE/Hosted image completing
  create/read/write/truncate/rename/fsync and both cross-platform readbacks;
- clean strict checker reports after the first AROS phase, host phase and final
  AROS phase;
- six Hosted and six native replay cases;
- four exact old-state and two exact new-state native outcomes;
- zero pending intent records in every accepted final image; and
- `guest_failure_requester=none` throughout.

The final checksum manifest validated in full. The compact retained result set
contains 156 files and a machine-readable eight-row requirement matrix, all
marked `PASS`.

## Consequences

The Mountable Alpha-0 objective is complete: the hardened mount/intent-log
core, portable VFS, host FUSE mount and MacAROS integration satisfy the stated
same-image operation, recovery and verification contract.

This is not an epoch-1 format freeze or a product-release claim. The report
explicitly records `hardware_claim=none`; native Apple hardware, persistent
reset durability, performance qualification, physical A500 testing and later
filesystem features remain separate milestones.
