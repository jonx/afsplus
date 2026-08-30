# ADR-054: Extract native-QEMU RAM and qualify intent-log replay

Status: Accepted for native MacAROS pre-hardware qualification

## Context

ADR-053 proved native AFS+ operations and a clean unload, but the retained-RAM
transport initially disappeared with QEMU. The target could check file content
before exit, while the host checker could inspect only the original input. That
left two distinct claims unproven under native MacAROS: the exact old/new result
of each modeled power cut and structural cleanliness after native replay.

Changing AROS or the filesystem handler to export test state would contaminate
the interface under test. Treating RAM as persistent media would overstate what
the QEMU transport guarantees.

## Decision

Run QEMU with a shared file-backed 512-MiB memory object when final-state
inspection is requested. After the guest has mounted, tested, dismounted and
unloaded AFS+, a host extractor:

- finds exactly one page-aligned version-2 retained-image descriptor;
- validates its FAT origin, bounds, flags, reserved bytes, handler padding and
  AFS+ payload magic;
- extracts exactly the described payload; and
- refuses ambiguous descriptors, trailing output or replacement of evidence.

The normal Alpha-0 QEMU gate now extracts the mutated payload and requires a
clean schema-5 checker report with zero pending intent records. Diagnostic
retained-RAM traces compile out by default so production qualification leaves
the descriptor reserved bytes canonical.

For replay, generate the same six deterministic fixtures as the Hosted gate.
Two embedded native probes independently require `old` or `new` content and no
temporary `HEAD.lock` name. Each case must then dismount and unload cleanly,
extract its final payload and pass the host checker. The compact evidence keeps
the before/after checker JSON, exact artifact hashes and QEMU report; the large
reproducible bundle and RAM intermediates are removed after hashing.

## Evidence

On 2026-08-30 all six cases passed native Apple-AArch64 QEMU execution:

- before any write, data before log, record over missing data and a torn log
  record each exposed exactly `old`, finished at generation 2 and had zero
  pending records; and
- a valid record before its barrier and after its barrier each exposed exactly
  `new`, finished at generation 3 and had zero pending records.

Every final checker report was clean with no warning or error. The ordinary
Alpha-0 operation image was also extracted after native create/write/truncate/
rename/fsync and was clean at generation 7 with zero pending records.

## Consequences

The modeled intent-log replay and strict final checker are now native-runtime
evidence, not merely host or link evidence. The method requires no change to an
AROS or MacAROS worktree and exercises the same off-tree handler artifact.

This does not turn `afsram.device` into durable storage. It does not prove that
`CMD_UPDATE` survives reset, inject a power loss into a running native guest,
or qualify Apple hardware. Those require a persistent native block transport
and a separate crash-control boundary.
