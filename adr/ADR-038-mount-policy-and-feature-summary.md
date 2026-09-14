# ADR-038: Explicit mount policy and identification feature summary

Status: Accepted

## Context

ADR-037 made a normal mount potentially write: a valid intent-log prefix is
replayed and checkpointed before the volume is returned. That behavior is
correct for ordinary operation but violates the forensic `NO_CHANGES`
contract. The intent log also occupied authoritative reserved blocks without
an executable compatibility decision at mount.

## Decision

Identification version 2 introduced three 64-bit feature summaries:
`COMPAT`, `RO_COMPAT`, and `INCOMPAT`. Bit zero in `INCOMPAT` is permanently
assigned to `org.aros.afsplus:intent-log`. Version-1 prototype identification
blocks remain readable; `log_slots > 0` is upgraded in memory to the same
incompatible bit.

Mount exposes four policies:

- `ReadWrite`: negotiate features, replay a valid log prefix, permit normal
  mutations;
- `ReadOnly`: expose the selected pre-replay checkpoint and report the valid
  pending-record count;
- `NoChanges`: the same pre-replay view with a tested zero-write contract;
- `Recovery`: replay mandatory recovery to disk, then expose the recovered
  checkpoint read-only.

Unknown `COMPAT` bits are ignored. Unknown `RO_COMPAT` bits refuse
`ReadWrite` and `Recovery` but permit `ReadOnly`/`NoChanges`. Unknown
`INCOMPAT` bits refuse every mount. The format decoder rejects a bit that is
present in more than one class and rejects disagreement between intent-log
placement and its feature bit.

`NO_CHANGES` scans and verifies the valid log prefix but never replays it,
flushes, advances a checkpoint, reclaims, or repairs. The mounted volume
reports how many intent records are pending, so tools cannot confuse the
pre-replay view with the latest fsync-durable namespace.

Intent-log record version 2 stores a timestamp on every logical operation.
Replay uses those timestamps for file and directory metadata. The decoder
still accepts prototype version-zero records, assigning their historical
zero timestamp because that information was never recorded.

## Consequences

Mount behavior is now explicit enough for a FUSE or MacAROS adapter to select
policy without duplicating recovery logic. Identification version 2 makes
older prototypes refuse new images instead of silently ignoring the journal.
Identification version 3 retains those summaries and adds the versioned
directory-key policy defined by ADR-052.
The compact masks are a mount-time summary, not the final rich feature
registry; dependencies, lifecycle metadata, and per-feature parameters may
still require feature records before epoch 1.
