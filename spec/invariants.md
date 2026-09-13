# Core Invariants

A conforming implementation must enforce these invariants.

<!-- toc -->

- [Allocation](#allocation)
- [Shared extents](#shared-extents)
- [User-data updates](#user-data-updates)
- [Objects](#objects)
- [Directories](#directories)
- [Journal](#journal)
- [Catalog](#catalog)
- [Checksums](#checksums)
- [Retention and reclamation](#retention-and-reclamation)
- [Failure and recovery boundaries](#failure-and-recovery-boundaries)
- [Catalog completeness](#catalog-completeness)
- [Change discovery](#change-discovery)
- [Adapter and cache obligations](#adapter-and-cache-obligations)

<!-- /toc -->

## Allocation

- every allocated physical block is accounted for as reserved metadata, a
  reachable allocation or deferred reclamation; multiple live data mappings
  require accounting by the active shared-extents feature
- no free bitmap bit may mark a reachable authoritative metadata block as free
- no extent may exceed volume bounds
- extent logical ranges for a file do not overlap

## Shared extents

When `org.aros.afsplus:shared-extents` is active, the volume-wide shared-extent
tree is the authority for data blocks referenced by multiple live mappings in
the selected checkpoint ([ADR-061](../adr/ADR-061-shared-extent-references.md)).

- the canonical tree contains exactly one maximal record for every physical
  sub-run with two or more live mappings, and no record for a sub-run with
  fewer than two mappings
- each record's reference count equals the number of live mappings covering
  every block in that record; records are ordered, non-overlapping, within
  allocatable bounds and merge adjacent runs with equal counts
- an extent without `EXTENT_SHARED` may not overlap a shared-tree record; an
  extent with `EXTENT_SHARED` but no overlapping record is legal and private
- direct-layout files may not participate in shared runs, and a file whose
  extent map carries `EXTENT_SHARED` may not collapse to direct layout
- shared data blocks are allocated exactly once in the bitmap and may not
  overlap metadata, authoritative free space or the reclaim queue
- dropping one of exactly two references removes the shared-tree record but
  does not retire or free the run; only dropping its final private mapping may
  enqueue it for deferred reclamation
- the shared-tree update and every extent-map update it describes become
  visible through the same checkpoint
- reference counts describe the selected checkpoint only; older selectable
  checkpoints are protected independently by deferred reclamation
- a non-zero shared-tree root or an `EXTENT_SHARED` flag without the feature
  bit is corruption; the feature bit with a zero root is the legal
  enabled-but-unused state

## User-data updates

[ADR-062](../adr/ADR-062-explicit-hybrid-data-updates.md) defines two explicit
per-file contracts:

- full data COW is the default and publishes replacement mappings only after
  their data satisfies the durability barrier
- private in-place update is an opt-in policy and may apply only to a
  non-extending write whose complete touched range is materialized and proven
  private
- a hole, unwritten mapping, shared marker, unresolved shared-reference state,
  or extension makes the complete operation COW
- no physical block with two or more live mappings is overwritten in place
- metadata remains COW regardless of the user-data policy
- a generation exposed after in-place overwrite may not be advertised as an
  exact historical byte version; exact-generation access fails explicitly
  when physical stability cannot be proved
- ignoring the in-place optimization and performing full COW is always a
  conforming, stronger fallback

## Objects

- object ID zero is invalid
- root object exists and is a directory
- live hard-link count matches reachable directory references, subject to orphan semantics
- object IDs do not change on rename
- when `org.aros.afsplus:orphan-directory` is active, object 2 is an unlinked
  directory root with link count one; each entry has a lowercase 16-digit
  hexadecimal name equal to its regular-file child ID, and supplies that
  child's sole directory reference
- no directory other than object 2 references object 2, and no ordinary
  namespace operation exposes object 2 or one of its entries
- an orphan cleanup checkpoint removes a bounded logical tail while retaining
  the entry, or removes the empty file entry and record; it never advances an
  external cursor independently of the selected checkpoint

## Directories

- every directory key is correctly normalized for that directory's case policy
- directory tree keys are ordered
- no directory entry points to an invalid object
- `.` and `..` are namespace conveniences, not required disk entries

## Journal

- transaction sequence is monotonic
- only committed transactions affect recovered authoritative state
- replay is idempotent or otherwise safely detectable
- checkpoint COW is the authoritative metadata transaction engine; group
  commit and the intent log feed that same engine rather than defining
  independent allocation or mutation semantics
- a valid committed log group is replayed atomically; successful fsync
  acknowledgement requires preserving its complete durable prefix on recovery
- an active intent-log version may advertise only operations its recovery path
  can validate and replay
- existing-file write/truncate records reference only replacement data made
  durable before the record and reject torn or missing content
- logged replacement extents are FREE in the base checkpoint, pairwise
  disjoint across the valid prefix and contained by the resulting logical file
  size; partial shrinking truncate records carry either the one required
  zero-tailed block or no data when the retained tail is sparse
- any active record version that can update existing file data requires the
  dedicated `org.aros.afsplus:intent-log-data-updates` incompatible feature;
  encountering such a record without the feature fails closed

## Catalog

- catalog is never authoritative
- catalog generation mismatch disables the fast path
- catalog records may not cause allocation or object lifetime decisions

## Checksums

- corrupted metadata is not silently accepted

## Retention and reclamation

- no block may be reused or discarded while a selectable or explicitly
  retained checkpoint can reach its previous contents
- an object generation identifies a revision, not a promise of retained bytes;
  exact-generation access returns that exact version or an explicit unavailable
  result; retention duration is governed by Q4 in
  [open questions](../implementation/open-questions.md)
- shared-reference counts and retained-checkpoint protection are independent
- growth preserves the configured emergency metadata headroom; destructive
  operations and bounded reclaim may consume it to restore forward progress

## Failure and recovery boundaries

- an operation rejected before publication must not change committed namespace,
  file contents or allocation ownership
- an I/O error during publication may have an uncertain durable outcome;
  recovery must select an allowed complete state, never a mixture
- a writer with uncertain publication state must reconcile that state or
  require remount before issuing another mutation from stale allocation state
- replay interrupted by another failure must preserve all acknowledged durable
  groups and must not replay an operation twice into a different semantic result
- normal mount uses bounded structural validation; exhaustive ownership and
  reachable-state validation belong to the checker
- no-changes inspection performs no media writes; salvage reports untrusted or
  missing information and never silently repairs the source
- repair reports affected identities, actions and discarded information;
  an inability to reconstruct data is not successful restoration

## Catalog completeness

When the optional catalog is advertised as current:

- it covers every namespace link through its advertised metadata generation,
  including links created before catalog construction began
- rebuild publication requires a consistent view or a proven enumeration and
  change-catch-up boundary; partial or interrupted builds are not current
- multiple names for one object remain distinct link records; object-oriented
  consumers deduplicate by stable object identity
- a stale or invalid catalog falls back to authoritative traversal

## Change discovery

When the optional persistent change stream is supported:

- only committed changes may be exposed as committed events
- enumeration and its saved cursor must form a provable handoff with no silent
  gap; sampling a cursor after an unconstrained traversal is insufficient
- expired, discarded or reset history requires explicit rescan
- a stream reset invalidates old cursors even when the filesystem UUID and
  numeric sequence values repeat; the cursor encoding is an M10 design gate
- event history is discardable and cannot be reconstructed from present state;
  catalog rebuild must not be described as rebuilding lost event history
- notification queues have bounded resource use and explicit overflow and
  cancellation outcomes; callbacks cannot expose partially initialized objects

## Adapter and cache obligations

- namespace links, open object identity and per-open state have separate
  lifetimes; removing a name cannot invalidate a surviving open reference
- resources used by in-flight I/O remain valid until that I/O completes;
  closing one handle does not imply all references have ended
- buffered and bypass I/O are coherent under the advertised access contract
- writeback and eviction preserve data-before-reference durability and cannot
  overwrite a durable metadata version with an uncommitted version
- resource exhaustion and writeback errors are observable; correctness cannot
  depend on unbounded dirty buffering or assumed retry fairness
- a host unable to represent security metadata preserves it or explicitly
  rejects the operation rather than silently weakening it

These clauses consolidate the accepted allocation/transaction contracts and
[design requirements](../docs/33-practical-filesystem-design-review.md).
They introduce no disk fields or API signatures. Query syntax, retention
policy, security evaluation and recovery/repair algorithms are not frozen by
these invariants. Their deciding experiments are in
[book-review qualification](../testing/book-review-qualification.md).
