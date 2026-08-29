# AFS+ Rust workspace

First executable prototype, implementing the "First contributor implementation
scope" of `../implementation/peer-review-prototype-plan.md`. Nothing here
freezes on-disk format decisions; prototype structures are marked as such in
the crate docs.

## Crates

- `afsplus-format` — on-disk structure encode/decode, CRC32C, explicit
  little-endian codecs. `no_std` + `alloc` (verified against a bare-metal
  target), zero dependencies. Bounds-first validation; every decoder rejects
  corrupted input via checksums and never panics on garbage.
- `afsplus-block` — the narrow block-provider trait plus test backends:
  memory, sparse host file, trace/accounting, deterministic fault injection,
  and power-cut simulation (write/flush log recording and crash-state
  enumeration with loss, reordering, and torn-write states).
- `afsplus-core` — mkfs, mount with A/B checkpoint selection and fallback,
  and the first writable COW transaction (create an empty file in the root,
  commit via the alternate checkpoint slot). Contains the shared
  reachable-state validator used by both mount and the checker (ADR-015).
- `afsplus-check` — verify-only checker (library + CLI) with human and
  versioned JSON output (ADR-025).

## Try it

```text
cargo test
cargo run -p afsplus-core --example mkimage -- demo.img
cargo run -p afsplus-check -- demo.img --json
```

## Status versus the plan

Done: workspace, block backends, smallest mountable image, first writable
transaction, crash matrix (power cut after every write/flush with subset,
reordering, and torn-write enumeration; every state must remount to exactly
the pre- or post-commit generation and pass the full checker).

Next: step 6, the region-allocation experiment that exposes the free-space /
metadata-COW recursion (architecture blocker 3). The bootstrap bump allocator
in the checkpoint record never reuses storage and must be replaced by that
work; the crash matrix must then be re-run, because block reuse is what makes
stale-but-valid content a real recovery hazard.
