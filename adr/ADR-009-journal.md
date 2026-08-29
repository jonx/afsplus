# ADR-009: Metadata transaction mechanism

Status: Reopened after PFS3/PFS4 Stage 0 review

## Context

The initial AFS+ draft selected metadata redo journaling because it offers bounded recovery and familiar transactional semantics.

Source-level review of PFS3 shows a compelling alternative. PFS3 reallocates changed metadata blocks, updates parent references recursively, writes the new metadata, and writes the root last as the logical commit point. Old metadata blocks are not made reusable until the new root has committed.

This architecture provides atomic metadata state changes without a conventional metadata redo journal and fits AFS+'s low-resource goals unusually well.

## Decision

AFS+ requires a transaction engine with:

- atomic metadata state transitions
- bounded crash recovery
- explicit write ordering/durability semantics
- bounded RAM requirements
- low write amplification
- safe handling of interrupted cleanup

The exact implementation is not yet frozen.

The leading design candidate is copy-on-write metadata plus alternating checksummed checkpoint records, described in `ADR-020-checkpoint-commit.md`.

A conventional redo journal remains a comparison candidate until the checkpoint design passes crash testing and write-amplification benchmarks.

## Consequences

`docs/08-transactions-and-journal.md` is no longer interpreted as requiring a conventional journal.

No epoch-1 on-disk journal format may be frozen until the checkpoint approach and at least one journal alternative have been tested under identical fault injection workloads.
