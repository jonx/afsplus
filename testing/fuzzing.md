# Fuzzing

> **ADRs:** none · **Spec:** none ·
> **Tests:** [`check-portable-c-fuzz.sh`](../tools/check-portable-c-fuzz.sh), [`check-rust-codec-fuzz.sh`](../tools/check-rust-codec-fuzz.sh) · **Milestones:** M01

<!-- toc -->

- [Required properties](#required-properties)
- [Target matrix](#target-matrix)
- [Rust codec gate](#rust-codec-gate)
- [Portable C corpus contract](#portable-c-corpus-contract)
- [Seeded semantic properties](#seeded-semantic-properties)
- [Legacy one-block reader oracles](#legacy-one-block-reader-oracles)
- [Typed caller and Unicode properties](#typed-caller-and-unicode-properties)

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

| Wire surface | Rust codec and caller coverage | Portable C path corpus | Remaining scope |
|---|---|---|---|
| Identification and retained checkpoints | Legacy and snapshot-bearing checkpoint targets plus raw and resealed mutations | Probe seed, header/block mutations | Add frozen reserved-field decisions |
| Object record and object-map node | File/directory/inline-symlink targets, empty/direct/tree-backed/in-place file variants, typed object-map payloads | Root lookup plus multi-leaf paths | Future object extensions and generic header policy ([Q13](../implementation/open-questions.md)) |
| Directory node | Generic tree-node target, typed payload admission and Unicode 16 comparison-key corpus | First/last ordinal in a 303-entry tree | Native/C Unicode interoperability and format freeze |
| Extent node and file data | Generic tree target, typed extent range/overflow checks, mounted sparse/shared-file byte oracles | Directory-to-file read seed; direct/sparse synthetic coverage remains in conformance | Portable C committed tree-backed file seed under reader conformance |
| Allocation-region metadata | Bitmap-page and region-descriptor targets, including a partial final page | None | Add portable repair-walker corpus |
| Intent-log record and referenced data | All five v3 operation types; three-record prefix, binding, sequence, data reuse/range/CRC and exact read-termination controls | Rust-built v3 write/truncate/create prefix scan plus final namespace lookup | Broader native recovery qualification |
| Snapshot registry, captured record, lifetime ledger and keys | Five direct targets; typed-tree ownership, ledger model and resealed cross-record snapshot checks | None | Portable C snapshot qualification |
| Legacy single-block directory, object map and retired list | Three direct targets with independent payload admission and decoded fields | None | Keep volume ownership and negotiated header/extension policy separate |
| Reclaim queue root, segment and table | Three direct targets; resealed cross-block count, generation, geometry, cursor and pending-total checks | None | Portable C reclaim qualification |
| Xattr record | None | None | Add when the portable reader exposes xattrs |
| Catalog record | None | None | Add with catalog implementation |
| Change-stream record | None | None | Add with change-stream implementation |

## Rust codec gate

`make rust-codec-fuzz-gate` exercises identification, checkpoint, typed-tree,
object-record, intent-log, bitmap-page, region-descriptor five snapshot leaf/key, three reclaim block snapshot-bearing checkpoint, inline-symlink, metadata object and three legacy one-block decoders. Each canonical seed must be accepted,
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
append IDs 6–7 and snapshot targets append IDs 8–12; reclaim targets append IDs 13–15 and snapshot-bearing checkpoints append ID 16; inline-symlink and object-metadata append IDs 17–18 and legacy directory/object-map/retired-list targets append IDs 19–21 under seed schema 1. On failure,
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

The snapshot-bearing checkpoint target starts with both registry and lifetime
roots present in the 112-byte payload. It independently checks every decoded
field, header/generation/UUID binding, root distinctness and structural ranges
for an 8,192-block, two-region fixture. Literal allocatable ranges are
`9..4096` and `4102..8192`; this oracle does not call the allocator geometry
predicate. Resealed controls cover all payload lengths through 120 bytes,
reserved-region and endpoint roots, equal/zero roots and generation mismatch.
The 96-byte form stays decodable and the legacy seed identity is unchanged.

The immutable feature bit is not an input to `Checkpoint::decode`.
[Checkpoint binding](../spec/snapshot-records.md#checkpoint-binding) requires
[mount selection](../crates/afsplus-core/src/mount.rs) to reject disagreement
between selected shape and feature bit without fallback. This format-only target
qualifies structural shape, not that caller negotiation or referenced tree
ownership. Common header verification is shared with other codec targets.

Inline-symlink and object-metadata targets use an explicit non-ASCII target and
a directory seed. Both exercise metadata-aware decoding, generic decoding and
borrowed symlink decoding against independent payload fields and predicates.
Valid file fixtures cover empty/direct/tree-backed and in-place-policy forms;
internal object type is explicitly rejected by these prototype codecs. Symlink
controls include UTF-8 continuation/overlong/surrogate/out-of-range forms, NUL,
empty targets, exact payload lengths, zero allocation fields, flags, reserved
bytes and unused tails. Borrowed target pointers must reference the input
payload; minimum and maximum encoder buffers are tested.

Payload byte 9 is documented reserved-zero and rejected by all object readers.
Generic file/directory header flags and unused-tail rejection have no explicit
normative rule in the cited object/header specification; symlink strictness does
not establish that rule for other types. Their admission policy requires a
separate format review with resealed header-flag, extended-payload and tail
fixtures, followed by an explicit accept/reject contract. This target does not
qualify caller geometry, policy-feature congruence or referenced extent trees.

The [format regression suite](../crates/afsplus-format/tests/roundtrip.rs)
requires undersized bitmap, region, directory, object-map, retired-list,
intent-log, reclaim-root, reclaim-segment and reclaim-table encoder outputs to return errors without panicking.
Bitmap, region, reclaim-segment and reclaim-table tests also check the exact
minimum successful buffer size.
These are encoder admission checks; they do not change valid on-disk bytes.
Additional object types and optional roots,
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

## Legacy one-block reader oracles

[legacy.rs](../fuzz/src/legacy.rs) supplies two-entry ordered seeds for the
legacy directory, object map and retired list. Independent payload extraction
checks exact counts and lengths, reserved-zero fields, ordering and invalid IDs
or retirement generations. Directory checks include bounded key/name lengths,
UTF-8 names, forbidden NUL/slash bytes and entry bounds. Truncation, resealed
fields/lengths and exact-minimum encoder output complement deterministic mutations.

Common-header verification is shared; no claim is made about an independently
implemented checksum parser. The directory oracle preserves the executable
legacy admission contract rather than applying current typed-tree Unicode-key
rules. Volume geometry, referenced ownership and generic header/tail decisions
are separate checks. Stable IDs 19–21 append fingerprints without changing
IDs 1–18; saved case-47 inputs for all three readers are replayed by the gate.


## Typed caller and Unicode properties

The [intent scanner](../crates/afsplus-check/tests/intent_scan_properties.rs)
uses three independently specified records. A valid stream is a positive control;
mutating only its middle binding, sequence, physical extent or content CRC must
retain exactly the first record and the expected presence or absence of a
diagnostic; this fixture does not assert the complete diagnostic text.
The exact device-read sequence proves that rejected data and later slots are not
read. Existing recovery matrices separately exercise publication and restart.

The [reclaim caller](../crates/afsplus-check/tests/reclaim_admission_properties.rs)
uses a root/table/segment chain whose blocks all decode individually. Ten malformed
relations test future generations, reference counts, region/device boundaries,
pending totals and a loaded cursor. The caller must reject the expected reason
within three reads and without writes; a valid chain preserves exact runs and
structure identities. These are exhaustive-checker properties, not permission to
walk the whole queue during normal mount.

The [typed mapping tests](../crates/afsplus-check/tests/typed_mapping_properties.rs)
put literal object, allocation and extent payloads into structurally valid tree
leaves. Widths, reserved bytes, descriptor slots/generations, physical bounds,
logical/physical overflow and zero-length extents reach the typed caller. Positive
controls check decoded fields; rejection uses one read without writes. The
[generic tree properties](../crates/afsplus-check/tests/tree_reader_properties.rs)
separately exercise ownership, generation, ordering, traversal bounds and cycles.

The [directory properties](../crates/afsplus-check/tests/directory_name_properties.rs)
combine literal spelling/key vectors, typed malformed payloads and mounted
collision/refusal/remount checks at 2/4/8/unlimited cache profiles. The bundled
[Unicode sources](../crates/afsplus-check/tests/data/unicode-16.0.0/sources.json)
pin the official Unicode 16.0.0 NormalizationTest and CaseFolding files by URL and
SHA-256, with their license. Tests run offline. All 19,965 normalization rows
exercise the five NFC identities through the public name API; names containing
NUL or slash must instead produce the filesystem's documented name refusal.
Every other scalar name (1,112,062 values) checks NFC against the corpus's
single-character inventory and the identity rule for omitted characters.
Full/default case folding uses the official C/F mappings, including identity
for absent mappings and exclusion of Turkic-only mappings. Its expected output
is normalized by the separately corpus-qualified NFC path; it does not use the
production folding dependency to construct expected folds.

These finite fixtures are deterministic source-controlled reproductions. A
failure names the mutation, corpus line or scalar; the vendored bytes and source
identity preserve the input without a network fetch or random generator. They
complement the codec mutation artifacts and semantic replay bundles, rather than
claiming all possible Unicode strings, proposed per-directory overrides,
native/C interoperability or epoch-1 format freeze.
