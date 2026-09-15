# Fuzzing

> **ADRs:** none · **Spec:** none ·
> **Tests:** [`check-portable-c-fuzz.sh`](../tools/check-portable-c-fuzz.sh), [`check-rust-codec-fuzz.sh`](../tools/check-rust-codec-fuzz.sh) · **Milestones:** M01

<!-- toc -->

- [Required properties](#required-properties)
- [Target matrix](#target-matrix)
- [Rust codec gate](#rust-codec-gate)
- [Portable C corpus contract](#portable-c-corpus-contract)
- [Seeded semantic properties](#seeded-semantic-properties)

<!-- /toc -->

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
| Allocation-region metadata | Bitmap-page and region-descriptor targets, including a partial final page | None | Add portable repair-walker corpus |
| Intent-log record and referenced data | One v3 seed containing all five operation types | Rust-built v3 write/truncate/create prefix scan plus final namespace lookup | Add multi-record sequence target |
| Snapshot registry, captured record, lifetime ledger and keys | Five direct headerless value/key targets with independent admission oracles | None | Add enclosing tree ownership and cross-record semantic properties |
| Reclaim queue root, segment and table | Three direct block targets with independent payload predicates | None | Add caller queue/geometry and cross-block consistency properties |
| Xattr record | None | None | Add when the portable reader exposes xattrs |
| Catalog record | None | None | Add with catalog implementation |
| Change-stream record | None | None | Add with change-stream implementation |

## Rust codec gate

`make rust-codec-fuzz-gate` exercises identification, checkpoint, typed-tree,
object-record, intent-log, bitmap-page, region-descriptor five snapshot leaf/key and three reclaim block decoders. Each canonical seed must be accepted,
re-encode and decode to byte-stable canonical form. The mandatory engine then
runs 4,096 stable cases per target using checksum-breaking bit flips,
CRC-resealed payload changes, short inputs, bounded multi-byte overwrites and
bounded extensions. Rejected inputs are normal; an accepted input must retain
the canonical round-trip property, and every decoder call is guarded so a
panic names the exact target and case.

The standalone crate has no network dependency and is excluded from the main
workspace so constrained builders need not compile qualification tooling.
The standard `make rust-gate` includes this separate workspace through
`rust-codec-fuzz-gate`. Its lockfile and seed-schema version keep case identities
stable: target IDs 1–5 and their seed bytes are unchanged; allocation targets
append IDs 6–7 and snapshot targets append IDs 8–12; reclaim targets append IDs 13–15 under seed schema 1. On failure,
the gate writes the last target/case before execution and stores the exact
input as a bounded `.afrf` artifact. Reproduce it with:

```sh
cargo run --manifest-path fuzz/Cargo.toml -- --replay failure.afrf
```

Confirmed regressions belong in `fuzz/regressions/`; every committed artifact
is replayed by the gate. `AFSPLUS_RUST_FUZZ_RUNS` raises the deterministic
per-target bound without changing any earlier case. A failure is preserved at
`build/rust-codec-fuzz-failure.afrf` by default; set
`AFSPLUS_RUST_FUZZ_ARTIFACT` to choose another durable path. Artifact publication
uses exclusive creation and file/directory durability barriers. An existing
artifact is an error and is never overwritten; failed publication may leave a
partial file, which is not a successful retained reproducer. Reading bounds both
the advertised file size and actual bytes read from the opened file.

Allocation seeds cover a three-page region with a partial final bitmap page.
Accepted bitmap inputs have an independently counted free-bit total, endpoint
mutation checks and a stable round trip. Region inputs must pass both decoding
and geometry/generation validation. CRC-resealed negative controls exercise
page counts, slot and generation bindings, valid-block counts and bitmap padding.
The gate replays exact saved inputs for tree, bitmap and region targets.

Snapshot leaf seeds are exact 32-byte registry, captured-root, lifetime and
ledger control values, plus an 8-byte big-endian key. These values have no CRC
header: their direct mutations deliberately bypass the enclosing tree checksum.
Independent wire-field predicates check exact length, reserved zero bytes,
generation/transaction ordering, physical ranges and control limits. Accepted
values require exact canonical bytes; registry exhaustion and half-open lifetime
membership have additional oracles. Fixed context is generation 17, 8,192 blocks
and lifetime start 101. Exhaustive short/extended lengths, every byte mutation
and numeric boundary controls complement 4,096 cases per target. Saved artifacts
for all five targets are replayed by the gate. This scope does not validate tree
ownership, snapshot visibility, cross-record accounting or crash consistency.

Reclaim targets use structured root, segment and table seeds. The root has
nonempty table, segment and inline areas, spare capacities and a nonzero cursor.
Independent payload predicates and field extraction check area bounds, counts,
run-end overflow, positive generations, totals and the cursor relationships
resolvable within one block. Resealed controls reach payload checks after CRC
admission; truncation covers every length of the canonical block. Additional
root variants exercise segment-only and inline-only heads. Common header
verification is shared infrastructure, not an independent checksum oracle.

These codecs reject nonzero payload reserved fields. They accept arbitrary
reference LBAs and header owner/flags; canonical re-encoding clears nonsemantic
header fields. Root totals are
checked arithmetically, not against referenced entries. Cursor bounds that require
loading a table or segment, physical geometry, ordering and queue ownership are
caller obligations, outside these direct targets. Reserved-field controls use CRC-resealed corruption so checksum failure cannot
mask payload rejection.

The [format regression suite](../crates/afsplus-format/tests/roundtrip.rs)
requires undersized bitmap, region, directory, object-map, retired-list,
intent-log, reclaim-root, reclaim-segment and reclaim-table encoder outputs to return errors without panicking.
Bitmap, region, reclaim-segment and reclaim-table tests also check the exact
minimum successful buffer size.
These are encoder admission checks; they do not change valid on-disk bytes.
Snapshot-bearing checkpoint seeds, additional object types and optional roots,
the other unassigned matrix surfaces require separate coverage.

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


## Seeded semantic properties

[The semantic generator](../tools/fuzz-semantic.py) drives the actual memory-image
runner with an independent object graph and byte-array oracle. Its versioned
xorshift64 generator produces reproducible seeds without depending on Python's
random implementation. Every sequence contains create, mkdir, cross-directory
file rename, unlink, empty-directory removal, sparse write, truncate, sync and
remount. Long names produce multi-leaf directories; random suffixes vary content,
block-boundary writes, shrink/grow, parent directories and object lifetime.
Directory rename, hard links, clones, snapshots and fault injection have separate
mutation-family gates and are not claimed by this scenario profile.

```sh
cargo build --offline -p afsplus-check --bin afsplus-scenario
python3 tools/test-fuzz-semantic.py
python3 tools/fuzz-semantic.py build/semantic-properties --seeds 1 7 42 --steps 96
python3 tools/afsptest.py replay build/semantic-properties/seed-7-prefix-96-cache-2
```

The driver probes the half-length and full sequence, each followed by a remount,
on 2/4/8/unlimited tree-cache profiles. The expected namespace and bytes are
computed before executing the filesystem, and identical semantic prefixes must
have identical expected states under every cache policy. Each case requires the
runner's exact-content comparison and raw/recovered checker evidence. The driver
rereads every published bundle to verify its retained bytes.

Admission bounds are 1–16 distinct unsigned 64-bit seeds, 64–256 operations per
sequence, 40 live files, eight live directories including root, depth at most two
below root, and 8,224 bytes per generated file. Each case uses a fixed 2 MiB image.
The existing scenario admission additionally checks aggregate bytes, geometry and
wire bounds. `--bundle-payload-mib` bounds the sum of retained bundle role bytes
(default 512 MiB, maximum 4 GiB); manifests, recipes and duplicate scenario JSON
are separate bounded overhead. On exhaustion the exact case input and incomplete
error record remain, but a complete runner bundle is not claimed. Runner timeout
or execution failure similarly leaves its input and recipe; it is not a passing
filesystem result. Resource limits are fixture admission, not OS memory guarantees.

Publication creates a fresh output directory, rejects non-ignored source overlap
and refuses overwrite. A recipe binds generator version/digest, seeds, prefixes,
cache profiles, source identity and executable digest. Each scenario is durable
before execution. Semantic failures retain a complete failing bundle and stop the
campaign; infrastructure/publication errors produce an incomplete record. Success
requires every case, and its completion record binds the recipe and case manifests.
Late barrier failure is an error even if a completion file is readable. Replay
uses the existing strict source/executable identity contract; retained source and
binary companions support later reconstruction.

The unit gate checks hand-written sparse/shrink/grow/rename examples, rejected
model operations, a golden generated scenario, all generator bounds/profiles,
source/output admission, failed execution, publication errors, payload exhaustion
and overwrite refusal. Qualification additionally replays retained cases in fresh
processes and requires an intentionally incorrect expected byte sequence to fail.
This state-machine corpus complements codec mutation and fault matrices; it does
not qualify ungenerated API families or arbitrary-length workloads.
