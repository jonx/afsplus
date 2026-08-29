# AFS+ Rust workspace

Executable prototype for the "First contributor implementation scope" and the
step-6 allocator experiment of `../implementation/peer-review-prototype-plan.md`.
Nothing here freezes on-disk format decisions; prototype structures are marked
as such in the crate docs.

## Crates

- `afsplus-format` — on-disk structure encode/decode (identification, A/B
  checkpoints with per-region descriptor records, object records with one
  direct data extent, single-block directories/object map, region bitmap
  pages and region descriptors, retired-block list), CRC32C, explicit little-endian codecs, region
  geometry. `no_std` + `alloc` (verified against a bare-metal target), zero
  dependencies. Bounds-first validation; every decoder rejects corrupted
  input via checksums and never panics on garbage.
- `afsplus-block` — the narrow block-provider trait plus test backends:
  memory, sparse host file, trace/accounting, deterministic fault injection,
  and power-cut simulation (write/flush log recording; crash states are all
  full-write subsets of the unflushed tail plus representative torn-write
  states — see the module docs for what the model does not cover).
- `afsplus-core` — mkfs, mount, transactions, and the region allocator
  experiment. Mount *selects* the newest structurally valid checkpoint
  without walking the filesystem, then reads only the object-map/root
  namespace and retired-list roots. Descendant objects are decoded on demand;
  allocation bitmaps are loaded page-by-page only as a transaction touches
  regions, while the exhaustive checker may load them all. Corruption is reported
  when the relevant root/descendant is read, never masked by falling back.
  The allocator keeps free-space state in multi-page region bitmaps selected
  through triple-buffered region descriptors. Every descriptor and logical
  page has three reserved generational slots, which breaks the bitmap-COW
  self-reference; freed blocks are quarantined via a retired
  list for one full generation before reuse. Per-transaction resource
  accounting (metadata/bitmap/flush counts, retired/promoted blocks, reclaim
  latency, allocator RAM) is collected from the start.
- `afsplus-check` — verify-only checker (library + CLI) sharing the core's
  selection/loading (ADR-015), plus the full invariant sweep normal mount
  does not run: link counts, orphaned objects, bitmap ⟺ reachability
  equality, quarantine invariants, and shadow verification of the retained
  older checkpoint (warnings). Human and versioned JSON output (ADR-025).

## Try it

```text
cargo test
cargo test -p afsplus-check --test measurements -- --nocapture   # cost table
cargo run -p afsplus-core --example mkimage -- demo.img
cargo run -p afsplus-check -- demo.img --json
```

## Status versus the plan

Done: workspace, block backends, smallest mountable image, writable COW
transactions (create with data, delete), crash matrix, fault injection, and
the step-6 allocator experiment with its mandatory quarantine workload:
create A → delete A (block X retired) → create B (X reused), with power cut
after every write/flush of both transactions and only the three allowed
recovery states accepted. A negative-control test replays a deliberately
mis-ordered commit (checkpoint before the metadata barrier) and proves the
matrix catches it.

Core Scale-1 is in progress. The shared, checksummed COW tree now has a bounded
lookup path, exhaustive verifier, and transactional multi-upsert engine. The
engine copies committed paths, reuses transaction-local staged nodes, splits
leaves/internal nodes, grows a new root, validates every level transition, and
reports mutation I/O/allocation statistics. The verifier checks
kind/owner/generation, separator ranges, exact per-child subtree counts, child
bounds, cycles, and duplicate child ownership. ADR-034 records the
experimental contract.

The current write overlay is a correctness vehicle and retains all dirty tree
nodes in RAM. Explicit staged-node spill/reload is still required for the
2/4/8-page tiny-cache qualification; this limitation is not hidden behind the
bounded lookup claim.

Next: COW deletion/redistribution/merge and root-height reduction, migration
of the object/allocation roots, extent trees beyond one direct extent,
directory B+ trees beyond one leaf, and the delta-log/spacemap
allocation alternatives — to be built only if this design fails on
correctness, write amplification, or scalability (the measurements test is
the baseline to beat).
