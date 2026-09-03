# Fuzzing

> **ADRs:** none · **Spec:** none ·
> **Tests:** [`check-portable-c-fuzz.sh`](../tools/check-portable-c-fuzz.sh), [`check-rust-codec-fuzz.sh`](../tools/check-rust-codec-fuzz.sh) · **Milestones:** M01

## Required properties

Every target must demonstrate:

- no crash or out-of-bounds access;
- no unbounded allocation from a corrupt length;
- no integer overflow;
- bounded work for a bounded input; and
- deterministic error classification where practical.

The seed corpus must derive from conformance images and retain intentionally
corrupt variants that have found bugs. A discovered failure is incomplete
until its exact bytes, engine options and expected classification are
replayable without the original workstation.

## Target matrix

| Wire surface | Rust codec target | Portable C path corpus | Remaining work |
|---|---|---|---|
| Identification and retained checkpoints | Canonical encode/decode seeds plus raw and resealed mutations | Probe seed, header/block mutations | Add frozen reserved-field decisions |
| Object record and object-map node | File-object and generic tree-node targets | Root lookup plus multi-leaf paths | Add every object type and optional root |
| Directory node | Generic tree-node structural target | First/last ordinal in a 303-entry tree | Add complete Unicode comparison-key tables |
| Extent node and file data | Generic tree-node structural target | Directory-to-file read seed; direct/sparse synthetic coverage remains in conformance | Add committed tree-backed file seed |
| Allocation-region metadata | None | None | Add with portable repair walker |
| Intent-log record and referenced data | One v3 seed containing all five operation types | Rust-built v3 write/truncate/create prefix scan plus final namespace lookup | Add multi-record sequence target |
| Xattr record | None | None | Add when the portable reader exposes xattrs |
| Catalog record | None | None | Add with catalog implementation |
| Change-stream record | None | None | Add with change-stream implementation |

## Rust codec gate

`make rust-codec-fuzz-gate` exercises identification, checkpoint, typed-tree,
object-record and intent-log decoders. Each canonical seed must decode,
re-encode and decode to byte-stable canonical form. The mandatory engine then
runs 4,096 stable cases per target using checksum-breaking bit flips,
CRC-resealed payload changes, short inputs, bounded multi-byte overwrites and
bounded extensions. Rejected inputs are normal; an accepted input must retain
the canonical round-trip property, and every decoder call is guarded so a
panic names the exact target and case.

The standalone crate has no network dependency and is excluded from the main
workspace so constrained builders need not compile qualification tooling.
Its lockfile and seed-schema version keep case identities stable. On failure,
the gate writes the last target/case before execution and stores the exact
input as a bounded `.afrf` artifact. Reproduce it with:

```sh
cargo run --manifest-path fuzz/Cargo.toml -- --replay failure.afrf
```

Confirmed regressions belong in `fuzz/regressions/`; every committed artifact
is replayed by the gate. `AFSPLUS_RUST_FUZZ_RUNS` raises the deterministic
per-target bound without changing any earlier case. A failure is preserved at
`build/rust-codec-fuzz-failure.afrf` by default; set
`AFSPLUS_RUST_FUZZ_ARTIFACT` to choose another durable path.

## Portable C corpus contract

`make portable-c-fuzz-gate` builds Rust images and records seven successful
operations: probe, root-object lookup, both extremes of a multi-leaf
directory, directory-to-file lookup/read, an intent-log scan that
content-verifies replacement extents and a final durable namespace lookup. The
recorder
stores only the observed blocks in `.afzf` sparse-device packets. Each packet
contains an operation header and fixed `(LBA, block)` records, so a complete
path is 12–48 KiB and missing data deterministically becomes an I/O error.

The mandatory engine applies 4,096 deterministic cases to every seed under
ASan and UBSan. Every packet-header byte and the first 128 bytes of each
observed block are mutated in both raw-checksum and CRC-resealed forms, so the
test reaches structural decoders rather than stopping only at checksum gates.
Block-wide single-bit, short-input, overwrite and multi-byte mutations follow,
with alternate cases resealed. The last seed and case are persisted before execution;
`--case N --artifact FILE` recreates the exact packet, and
`afsplus-fuzz-replay` prints symbolic operation results plus the diagnostic
stage and LBA. `AFSPLUS_FUZZ_RUNS` increases the bound without changing case
identity.

The target exports `LLVMFuzzerTestOneInput`, and the gate additionally runs
native libFuzzer when the compiler installation provides its runtime. Missing
libFuzzer support is not a waiver: the deterministic sanitizer engine is the
portable baseline.
