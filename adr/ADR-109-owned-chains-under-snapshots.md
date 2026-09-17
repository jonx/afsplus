# ADR-109: A retained snapshot keeps the owned chains it captured

Status: Accepted
Amends: ADR-101, ADR-108

## Context

[ADR-101](ADR-101-security-preservation-container.md) refused to mount a
volume with both security descriptors and persistent snapshots, and
[ADR-108](ADR-108-extended-attributes.md) refused to set attributes on a
snapshot volume. Both waited for the snapshot lifetime ledger
([ADR-071](ADR-071-snapshot-lifetime-prototype.md)) to own chain segments: a
retained view keeps the object record it captured, and the record names a
chain the live side may retire.

Reading the allocator settled the question. The ledger is not told about
kinds of blocks. Every allocation and every retirement made while a
transaction is in its namespace phase is recorded, as a physical run with its
birth and retirement generations. Chain segments are allocated and retired in
that phase, so the ledger has owned them since the first chain was written on
a snapshot volume. What was missing was proof and a way to read.

## Decision

1. No format change. A chain segment is a namespace metadata block: its
   lifetime is a ledger run, a retained view whose generation falls inside
   that lifetime keeps it, and it returns to ordinary quarantine with the
   last view that needs it.
2. The mount refusal of ADR-101 and the write refusal of ADR-108 are removed.
   The formatter offers security descriptors and persistent snapshots
   together.
3. The checker's walk of a retained view proves both chains of every record
   the view holds, at the view's generation, and claims their segments as
   historical metadata. A captured segment that is damaged, reused or outside
   its ledger lifetime is a checker error, and the rule that a metadata
   block's header generation equals its ledger birth covers chain segments.
4. The core reads the captured state: `snapshot_attribute`,
   `snapshot_attribute_names` and `snapshot_security_descriptor`, each through
   the record the view holds and bounded by the view's generation.
5. The core returns a captured descriptor to any caller that holds the view.
   Who may open a view and read historical security metadata after the live
   permissions changed remains host policy and stays open in Q5.

## Compatibility classification

No format change and no feature identity. An image written before this
decision with either feature is unchanged; an image with both features did
not mount before and mounts now.

## Format-change procedure

Not a format change. The proof is in
`crates/afsplus-check/tests/chain_snapshots.rs`: a view created over two
objects that each own a three-segment attribute chain and a three-segment
descriptor chain still returns every captured byte after the live side
cloned one object, replaced and removed its attributes, replaced its
descriptor, deleted the other object and wrote every free block three times,
across remount; the twelve captured blocks are byte-identical, owned by the
ledger and by no live object; explain reports no problem and the image diff
none; after the view is deleted the captured blocks are released or still
ledger-owned pending the bounded reclaim scan, never allocated without an
owner; a broken captured segment of either kind, reached only by the view,
fails the checker; and at every modeled power cut of replacing the set and
the descriptor under a view (1,251 states) the view returns the captured
bytes and the live side one of its three legal states, with a clean checker
verdict. Without the historical chain walk of decision 3 the checker test
fails.

## API contract consequences

- Three read operations on a snapshot handle. The outer `None` is an object
  the view does not hold; the inner one an object without the attribute or
  the descriptor.
- `mkfs_with_snapshots_and_security_descriptors` formats a volume with both
  features.
- Backup transport of captured descriptors and attributes is not decided
  here.

## Consequences

- Attributes and descriptors work on every volume.
- A retained view can hold up to 17 blocks per chain per captured object
  beyond what the live side uses, accounted like any other retained block.
- A third kind of owned chain inherits all of this without ledger work.
