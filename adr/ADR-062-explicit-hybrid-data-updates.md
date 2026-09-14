# ADR-062: Explicit hybrid policy for committed user-data updates

Status: Accepted as the epoch-1 data-update architecture; the persistent policy encoding is assigned by ADR-065
Amended by: ADR-079
Amends: ADR-020, ADR-061

## Context

Metadata COW does not determine whether a rewrite may overwrite committed
user-data blocks. Full data COW preserves the bytes reached by an older
checkpoint, but random database/VM-style rewrites allocate, retire and
fragment storage. In-place update avoids that work but an older valid
checkpoint can then expose newer or torn file bytes after a crash.

ADR-061 supplies the required shared/private distinction: a physical range
with multiple live mappings is represented in the shared-extent reference
tree and cannot be overwritten in place. Open question Q1 therefore became
measurable without weakening reflink correctness.

The runtime prototype compares the policies on fresh 4 KiB-block images. On
1,000 random rewrites across a 16 MiB file, conservative private in-place
update reduces device writes from 26,972 to 8,000, allocations from 24,870 to
3,000, retirements from 21,936 to 2,000, and final fragmentation from 1,767
extents to one. On a 32-page database hot set it reduces writes by 11%,
allocations by 40%, retirements by 50%, and final fragmentation from 31
extents to one. Append and first writes to shared blocks remain COW and are
identical in both modes. Full results and limitations are in the
[Q1 bake-off](../implementation/data-policy-bakeoff.md).

The power-cut and injected-error tests also make the semantic cost explicit.
Full COW recovers exact old or exact new contents. With private in-place
update, metadata remains valid, but an older checkpoint can read old, new or
torn bytes inside the overwritten range. Even a returned I/O error does not
imply byte rollback once the data write reached the device.

## Decision

AFS+ uses an **explicit per-file hybrid policy**.

1. Full data COW is the creation default. It retains the exact old-or-new
   content contract while the corresponding checkpoint generation is
   retained and its blocks have not otherwise left the retention contract.
2. A filesystem-neutral policy API may opt a file into private in-place
   updates. The choice is persistent per file; workload detection never
   changes durability semantics automatically.
3. An in-place operation is eligible only when the complete requested write
   is non-extending and every touched block is materialized, mapped and proven
   private. A hole, unwritten extent, shared marker, unresolved reference
   state, or extension sends the complete operation through data COW.
4. A shared physical sub-run is always COW while two or more live mappings
   exist. A stale conservative shared marker also forces COW; the optimization
   fails closed.
5. Opting into in-place update explicitly relinquishes historical byte
   stability for affected generations. An exact-generation API returns
   `GENERATION_NOT_AVAILABLE` rather than labeling possibly overwritten bytes
   as an exact historical version.
6. Metadata remains COW in both modes. The in-place policy changes user-data
   versioning, not the checkpoint transaction engine or allocation metadata.

The runtime `DataUpdatePolicy` switch is qualification machinery, not the
shipping per-file API. It resets to full COW at mount until the persistent
policy representation exists.

## Compatibility classification

The policy has **compatible fallback semantics**: an implementation that does
not perform private in-place updates may always use full data COW, which is a
stronger durability behavior and does not change visible file contents.

No numeric object flag or feature bit is assigned by this ADR. The later
format/API change must store the policy somewhere unknown writers preserve.
If that cannot be guaranteed, the encoding must use a compatibility class
that prevents such writers from silently dropping the choice. This ADR fixes
the semantics, not an unsafe placeholder bit.

## Rejected alternatives

- Full COW for all private writes is rejected as the sole epoch-1 policy: the
  measured random-write allocation and fragmentation costs are too high for
  named database and VM workloads.
- In-place update for all private files is rejected: it would silently remove
  useful old-generation stability from source trees, package stores and
  content-inspection consumers.
- Automatic hot-file detection is rejected: durability behavior must not
  depend on access history, and the experiment provides no interoperability
  contract for such detection.

## Validation and remaining freeze gates

The accepted architecture is covered by:

- default-policy, private eligibility and remount-reset tests;
- unaligned multi-block update and shared/extension fallback tests;
- an exhaustive modeled power-cut matrix with distinct COW and in-place
  semantic oracles;
- deterministic post-data metadata-I/O failure coverage;
- optimized random, database-hotset, append and reflink workloads followed by
  the exhaustive checker.

Before M14 freezes an encoding, AFS+ still requires the persistent per-file
representation and filesystem-neutral API, exact-generation API tests,
low-space and real-storage CPU/RSS qualification, and a portable-C
implementation of the same fail-closed rule.

## Consequences

The format has one predictable safe default and an explicit high-performance
mode for overwrite-heavy files. Callers can choose the tradeoff; AFS+ does not
pretend that metadata consistency implies historical data atomicity.

Q1 is closed. Q4 remains open because retention duration and handle lifetime
still need a concrete consumer, but it may no longer promise exact bytes for
an in-place generation.
