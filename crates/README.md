# AFS+ Rust workspace

Executable prototype for the "First contributor implementation scope" and the
step-6 allocator experiment of `../implementation/peer-review-prototype-plan.md`.
Nothing here freezes on-disk format decisions; prototype structures are marked
as such in the crate docs.

## Crates

- `afsplus-format` — on-disk structure encode/decode (identification, A/B
  checkpoints, object records with one direct data extent, shared typed AFST
  nodes for object maps/directories/allocation records, region bitmap pages
  and descriptors, retired-block list, plus transitional legacy codecs), CRC32C,
  explicit little-endian codecs, region
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
  self-reference; freed block runs are quarantined
  in a segmented reclaim queue (ADR-036): appends seal into immutable
  segment/table blocks, each transaction reclaims a bounded block budget
  from the head, and a persistent cursor resumes after crashes. Blocks a
  transaction allocates and discards before publication are released back
  to free immediately. Per-transaction resource
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
lookup path, exhaustive verifier, and transactional upsert/delete engine. The
engine copies committed paths, reuses transaction-local staged nodes, splits
leaves/internal nodes, merges or redistributes underfull siblings, grows or
collapses a root, validates every level transition, and reports mutation
I/O/allocation statistics. The verifier checks
kind/owner/generation, separator ranges, exact per-child subtree counts, child
bounds, cycles, and duplicate child ownership. ADR-034 records the
experimental contract.

Modern-host mutation retains final dirty images for speed. The same engine now
has a constrained mode that spills only freshly allocated, unreachable images
and enforces staged-image budgets of 2, 4, and 8 pages. Recursive descent keeps
only compact coordinates, re-reads parents on unwind, and RAII instrumentation
proves that insertion and merge-heavy deletion retain at most two decoded or
derived full nodes at once. The per-final-node overlay index and caller-owned
operation batch remain proportional to the mutation, so this is a bounded
page-cache result rather than a claim of constant total RAM.

The checkpoint object-map root and every newly formatted directory now point
to typed shared trees rather than legacy one-block codecs. Normal mount checks
only their roots; `stat` and name lookup descend on demand; create/delete
publish both mutations through the existing metadata barrier/checkpoint
protocol; and the checker visits and claims every tree node. Typed 1,001-entry
object-map and 1,000-entry directory tests cross page boundaries. An explicit
release qualification builds and streams 100,000 typed directory entries
through a height-three tree under an eight-page staged cache; every original
name, comparison key, child ID, and type hint is validated without collecting
the directory in the iterator. An end-to-end 300-entry namespace test also
exceeds the old directory limit,
checks the image, remounts it, and verifies enumeration/lookup. Existing
power-cut, fault, and reuse matrices cover the ordinary publication path.
An additional boundary test discovers the exact root-directory 1→2 height
transition, crash-qualifies its split, then deletes the boundary entry and
crash-qualifies the merge/root collapse back to height 1.

The namespace API now addresses arbitrary directories by stable object ID and
implements create, mkdir, unlink, empty-directory removal, file hard links,
and same/cross-directory rename. Rename updates all affected COW roots in one
checkpoint transaction, preserves the moved object ID, rejects moves into a
descendant, and passes an every-write/every-flush power-cut matrix. Hard-link
counts are checked against the complete namespace; storage survives the first
unlink and is retired only after the final link disappears.

Extent maps now support direct and multi-level representations, sparse range
writes, truncate, and unwritten preallocation. Allocation-root updates load
retained records on demand and mutate only dirty paths; a 145-region power-cut
matrix crosses its first height boundary, while a sparse 1 TiB qualification
covers format, bounded mount, commits, and exhaustive checking. Remaining
post-Scale-1 work includes atomic replacement/orphan handling and the broader
workload suite. Delta-log/spacemap alternatives are built only if the measured
bitmap design fails on correctness, amplification, or scale.

Reclaim Scale-2 replaces the single-block retired list with the ADR-036
queue. Entries are runs, so a large truncate or unlink costs a handful of
entries; a 600-block COW rewrite, a 400-block truncate plus 400-block bulk
unlink, and an ignored ~1.9M-block preallocate/unlink qualification all
quarantine and drain in strictly bounded reclaim steps (the millions-scale
drain completes in ~0.2 s release, under 32 structure writes per step).
Dedicated crash matrices cover a sealing transaction, a batch that consumes
and retires a whole segment, and a mid-run cursor advance that must resume
after remount; the tiny-caps drain test forces segment *and* table sealing
and returns the volume to a one-block steady state where only the previous
queue root cycles through quarantine.

ADR-035 defines the authoritative allocation-root representation. The shared
engine accepts either the ordinary transaction allocator or a permanently
allocated triple-version node pool, avoiding free-space self-reference while
keeping one AFST implementation. Checkpoints now publish its root and total
free count instead of inline per-region records.
