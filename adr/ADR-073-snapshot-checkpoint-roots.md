# ADR-073: Bind snapshot roots through an incompatible checkpoint extension

Status: Accepted
Amended by: ADR-111
Amends: ADR-072

## Context

ADR-072 defines portable registry and lifetime records and their incompatible
feature identity. The common checkpoint must bind both trees atomically before
any writer can expose persistent snapshots. The existing 96-byte payload and
reserved flags retain their meanings for feature-absent images.

## Decision

Extend the checkpoint payload to 112 bytes on snapshot-enabled volumes. Append
the registry-root LBA at payload offset 96 and lifetime-ledger-root LBA at 104,
both little-endian u64. Both roots are nonzero, distinct and within allocatable
geometry. The common checked header covers the extended payload. The reserved
flags word at offset 80 remains zero.

A feature-absent checkpoint uses exactly 96 payload bytes. A snapshot-enabled
checkpoint uses exactly 112 and contains both roots, including when no views
are registered. Reject every intermediate/oversized length. The codec exposes
the extension explicitly; it never guesses roots from padding.

After structural checkpoint selection, validate congruence between the selected
payload and the immutable snapshot feature bit. A mismatch rejects the selected
state; it never triggers silent fallback to older namespace state. Reading
registry/ledger descendants is separate from selection. Mount must validate
bounded root/control state and keep exhaustive ownership checks in maintenance.

Creation, deletion and lifetime updates publish both roots through the common
COW commit tail with namespace/allocation state. Housekeeping tree nodes enter
ordinary quarantine; namespace lifetimes follow ADR-071. The snapshot registry
captures namespace roots only and never pins allocation-root pool images.

## Compatibility and staged integration

The extension requires INCOMPAT bit 2. Old readers reject it before writes.
Feature-absent images retain byte-compatible checkpoint output. Adding the
codec extension alone does not add the snapshot bit to mount's supported mask.
The full writer, immutable readers and crash/reclaim gates must precede that
capability change.

## Validation, repair and resources

Require exact 96/112-byte payload round trips, offsets, zero/distinct root
checks, every invalid length, invalid geometry, valid-CRC feature mismatches,
and no fallback on a mismatched newest selected checkpoint. Verify legacy
images retain their bytes and unaware Rust/C readers reject the feature.

Two root pointers add 16 bytes within the existing checkpoint block, with no
additional checkpoint write or flush. Registry/ledger tree I/O and reserves
require integrated measurements. A corrupt selected root is an explicit
salvage finding; repair must preserve the source and cannot discard a promised
view implicitly.
