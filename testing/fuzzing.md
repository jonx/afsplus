# Fuzzing

> **ADRs:** none · **Spec:** none ·
> **Tests:** [`check-portable-c-fuzz.sh`](../tools/check-portable-c-fuzz.sh), [`check-rust-codec-fuzz.sh`](../tools/check-rust-codec-fuzz.sh) · **Milestones:** M01

<!-- toc -->

- [Required properties](#required-properties)
- [Target matrix](#target-matrix)
- [Rust codec gate](#rust-codec-gate)
- [Portable C corpus contract](#portable-c-corpus-contract)
- [Seeded semantic properties](#seeded-semantic-properties)
- [Generated operation families](#generated-operation-families)
- [Executable surface audit](#executable-surface-audit)
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
This version-1 profile moves files only. [Generated operation families](#generated-operation-families)
cover deferred windows, persistent snapshots, hard links, symlinks, directory
rename, clones and protection. Fault injection belongs to the separate
mutation-family gates of [crash testing](crash-testing.md).

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

## Generated operation families

Generator version 2 of [the semantic generator](../tools/fuzz-semantic.py) adds
nine families selected with `--family`. Each family uses the version-1
xorshift64 sequence, seed and length bounds (1–16 distinct seeds, 64–256
operations), half and full prefixes followed by a remount, the four cache
profiles, durable publication and the recipe/result records above. The
version-1 generator and its golden scenario keep their bytes. A family recipe
adds `family`, the scenario version and the applied negative control.

| Family | Scenario | Generated operations | Independent oracle |
|---|---|---|---|
| `window` | version 5 | create, mkdir, write, truncate, rename, unlink, rmdir, sync, remount, `window_write`, `window_truncate`, `window_fsync`, `window_commit` | Committed byte model plus staged work: fsync acknowledges every staged operation, commit publishes all staged work, remount keeps the acknowledged prefix and drops the rest |
| `snapshot` | version 7 | create, mkdir, write, truncate, file and directory rename, unlink, rmdir, sync, remount, `snapshot_create`, `snapshot_open`, `snapshot_inspect`, `snapshot_close`, `snapshot_delete` | Object graph with generations, object IDs, timestamps, link counts and single-block allocation; each view is copied at creation and compared as complete `expected_snapshots` metadata |
| `namespace` | [version 9](developer-harness.md#linked-namespace-replay-bundles) | create, mkdir, write, truncate, rename of files, symlinks and directories, `link`, `symlink`, `unlink_symlink`, `clone_file`, `clone_range`, `set_protection`, unlink, rmdir, sync, remount | The same object graph projected to paths, kinds, link counts, protection, hard-link alias ordinals, bytes and opaque symlink targets |
| `replace` | version 9 | create, mkdir, write, truncate, rename, `link`, `rename_replace`, unlink, rmdir, sync, remount | The linked projection: the source takes the replaced name, a victim with further links keeps them, and a final-link victim leaves the namespace with its storage |
| `orphan` | version 9 | create, mkdir, write, truncate, rename, `orphan_file`, `rename_replace_orphan`, `cleanup_orphan`, unlink, sync, remount | The linked projection plus the reserved-directory entry count and the byte total of the objects it names |
| `space` | version 9 | create, mkdir, write, truncate, rename, `preallocate`, `preallocate_bounded`, `write_bounded`, `truncate_bounded`, `set_data_policy`, `restore_metadata`, `set_protection`, unlink, rmdir, sync, remount | The linked projection with the persistent per-file policy flag and the normalized allocation coverage of every file |
| `batch` | version 9 | create, mkdir, write, truncate, `batch`, `window_batch`, `window_fsync`, `window_commit`, unlink, rmdir, sync, remount | The linked projection after one atomic transaction per group, plus the reserved-directory entry count and the byte total of the objects it names; a window publishes its acknowledged prefix at remount and every staged group at commit |
| `maintenance` | version 9 | create, write, truncate, unlink, `reclaim_step`, `snapshot_maintenance_step`, `snapshot_create`, `snapshot_open`, `snapshot_inspect`, `snapshot_close`, `snapshot_delete`, sync, remount | The linked projection and every captured view, both invariant across every maintenance step |
| `captured` | version 9 | create, mkdir, write, truncate, rename, `link`, `symlink`, `clone_file`, `clone_range`, `preallocate`, `set_protection`, unlink, rmdir, the five snapshot commands, sync, remount | Captured views of multi-block files, hard links, symlinks, clones, cloned ranges and reservations, with exact bytes and metadata and normalized coverage |

Every window sequence starts with a ladder: an acknowledged group survives a
remount that loses a later write; two acknowledged groups and an unacknowledged
write are published by commit; an unacknowledged truncate is lost at remount.
Direct mutations and `sync` never run while a window is open. Window truncates
change the visible size and window writes carry data, so every staged call opens
or extends the window. A window holds at most 24 staged operations, eight
unacknowledged operations per group and six log records of the eight-slot
fixture; fsync and commit without a window are admitted no-ops. At most 16 files
of 8,224 bytes and four directories are live.

The snapshot ladder keeps a handle open across a write and a file move, inspects
and closes it, moves a directory across parents, captures sparse growth of a
file that is unlinked later, remounts, reopens the second view, deletes the first
and removes a directory. Snapshot labels are unique; inspect and close use open
handles; delete requires a closed handle; remount closes every handle. At most six
views, twelve files and six directories are live, at depth two with short names.
File data stays in logical block zero, so allocation is exact: one 4 KiB range
once that block is written and none before. The model publishes one generation
per mutation (a same-size truncate, an empty write or unchanged protection
publishes none), captures the current generation before the registry commit of
`snapshot_create`, assigns snapshot IDs from 1 without reuse and object IDs from
16, stamps operation index i with i + 1 seconds, and reports 4,096 allocated bytes
for the root and zero for other directories.

The namespace ladder writes through a hard link in another directory, creates a
symlink, sets protection, clones the file, clones the unaligned range at source
offset 1 to destination offset 4,097, rewrites the source, moves a directory and
a symlink across parents, unlinks one link and the symlink, truncates the clone,
links it again and creates a non-ASCII target. The random suffix keeps at most 48
names, eight directories at depth three and files of 8,224 bytes. CloneRange uses
distinct objects, matching block residues and source offsets 0, 1, 4,095, 4,096
or 4,097 inside the source. A CloneRange destination takes the source blocks of
every complete block of the requested range, so a source hole stays a hole and a
source reservation stays a reservation, and its at most two partial boundary
blocks are copied into private storage whatever the destination held there. The
source keeps its logical bytes and its modification time, and its change time
moves exactly when a mapped block of the shared range gains the shared flag: a
call that shares no complete block, or one over a range every block of which
carries the flag already, leaves the source record alone. A block a write
replaces, and the partial tail a shrinking truncation rewrites, return to
private storage. Directory moves stay outside the moved subtree and
names are fresh. CloneFile gives the new object the source bytes, size and
protection with one link; the oracle follows the executable
[CloneFile](../crates/afsplus-core/src/volume.rs) behavior, because
[clone semantics](../docs/32-reflink-clone-semantics.md#3-clonefile) leave
metadata inheritance to the API contract.

The `replace` ladder replaces a plain file, replaces a hard-linked victim whose
object survives under its other name, replaces across two directories and reads
the replacing bytes after a remount. Source and victim always name distinct live
file objects, and the victim label names the exact entry being replaced.

The `orphan` ladder moves a contiguous file, an empty file and a replacement
victim into the reserved directory, cleans each one, carries reserved entries
across a remount and leaves one entry pending. A cleanup step removes whole
extent records from the logical end and, once the layout is empty, the entry and
the object record in the same call ([ADR-066](../adr/ADR-066-bounded-orphan-directory.md)).
The family sets the cleanup budget to two extent records and generates only
orphans whose complete layout fits one budget, so one call completes each one.
The observation reports the reserved-directory count from the volume and sums
the sizes of the objects the executed case placed there; an entry outside that
set is an observation error.

The `space` ladder reserves capacity past the written blocks, writes into one
reserved block, reserves under exact block and record budgets, opts a file into
the persistent policy and back out, restores archived protection and
timestamps onto a file and a directory, writes across a block boundary under its
exact touched-block budget, shrinks under the budget of the blocks it retires
and grows under the admitted budget floor. A reservation keeps the logical size,
the bytes and the modification time, and it makes its blocks read as zeros. A
budgeted edit reaches the same bytes, size and coverage as its unbudgeted form:
a write budgets the logical blocks it touches, and a shrink budgets the blocks
it releases plus the private rewrite of a written partial tail. A record budget
counts stored extents, whose boundaries follow physical placement, so the
generator uses the admission maximum for it. The volume carries the `COMPAT`
data-policy feature for this family alone.

The `batch` ladder creates a group, moves and deletes in one group, replaces
inside a group, stages a window group that an fsync acknowledges, loses an
unacknowledged group at a remount and publishes the acknowledged prefix. A group
holds one to sixteen members, and a member never names a label its own group
created. Window groups stage creates, moves, deletes and replacements. The
ladder continues with three staged final unlinks: an acknowledged group sends
its object to the reserved directory at a remount that loses the group behind
it, whose file keeps its name; a commit sends a third object there; a staged
create that its own window deletes leaves neither a name nor a reserved entry;
and a staged replacement sends its final-link victim to the reserved directory.
The model resolves each staged unlink at publication, because the deciding fact
is whether the same window created the object
([ADR-066](../adr/ADR-066-bounded-orphan-directory.md)). A window holds at most
six staged groups and eight unacknowledged member operations, and a label a
window group names is spent, so a later operation never names it again.

The `maintenance` ladder captures every view before the first step, produces
reclaimable capacity, runs reclaim and snapshot-maintenance steps, remounts and
runs further steps, reopens and inspects a view and deletes another. The model
captures no view after a step, because a step may publish a checkpoint whose
generation the model leaves open. Termination is the fixed point that the
namespace, the bytes and every captured view reach: repeated steps change
none of them, and the case publishes its verdict after a final remount.

The `captured` ladder builds a multi-block file, a hard link, a symlink, a clone
and a reservation, clones two complete blocks of a second file into the first,
captures a view, changes each of them independently, clones the same source
range again and clones a boundary-only range, captures a second view, remounts
and reads both. Captured entries compare exact bytes, exact metadata and
normalized coverage, so the captured metadata of a CloneRange source compares
its change time.

Version 9 compares a normalized logical allocation coverage. Every observed
range is expressed in logical bytes, and two adjacent ranges merge into one
interval when their unwritten flag agrees, so the value is the list of maximal
logical intervals. Coverage depends on which logical bytes hold reserved
capacity and whether those bytes read as zeros; extent record boundaries and
physical placement are outside it. Intervals are block aligned, ascending and
disjoint, and two neighbouring intervals always carry different flags. Both the
live linked records and the captured view entries carry this form, and every
expected file entry carries it, so the admission layer refuses an absent
coverage.

```sh
cargo build --offline -p afsplus-check --bin afsplus-scenario
python3 tools/test-fuzz-semantic.py
python3 tools/fuzz-semantic.py build/window-properties --family window --seeds 1 7 42 --steps 96
python3 tools/fuzz-semantic.py build/window-properties --replay
python3 tools/fuzz-semantic.py build/window-control --family window --negative-control window-byte
```

The driver computes every case before creating the output directory and refuses
a campaign whose `expected` or `expected_snapshots` differ between cache profiles
for one seed and prefix. Each case publishes a complete runner bundle. `--replay`
reruns every retained case of a campaign in a fresh `afsptest.py replay` process
and requires the recorded manifest digest and verdict. `--negative-control`
changes exactly one expected value in the first case where it applies, records
that case in the recipe and exits zero only when the campaign stops at that case
with its failing bundle retained.

| Control | Family | Wrong expected value |
|---|---|---|
| `window-byte` | window | First byte of a file published by window commit or replay |
| `snapshot-entry` | snapshot | First byte of the first captured file with data |
| `link-count` | namespace | Link count of a multiply linked file, minus one |
| `symlink-target` | namespace | Symlink target with one appended character |
| `protection` | namespace | Lowest protection bit of the first entry with nonzero protection |
| `clone-byte` | namespace | First byte of a CloneFile or CloneRange destination |
| `directory-rename` | namespace | Final component of a moved directory |
| `replaced-byte` | replace | First byte of a file that took a replaced name |
| `orphan-count` | orphan, batch | Reserved-directory entry count |
| `orphan-bytes` | orphan, batch | Byte total of the objects the reserved directory names |
| `reservation` | space | Length of the last allocation interval of the first file with coverage |
| `policy-flag` | space | Persistent data-update policy of the first file |
| `bounded-byte` | space | First byte of a file a budgeted write or truncation edited |
| `batch-path` | batch | Final component of a path a batch created |
| `maintenance-entry` | maintenance | First byte of the first captured file with data |
| `captured-coverage` | captured | Length of the last interval of a captured file's coverage |
| `clone-changed` | captured | Captured change time of a CloneRange source whose layout the call marked, plus one second |

The unit gate checks window, object, version-9 and captured-view model examples,
golden family scenarios, required operations and bounds at seeds 0, 1, 7, 42 and
2^64 − 1, profile independence, control placement, family recipes, control
reproduction, fresh-replay binding, and one real-runner pass plus every failing
control. Generated families run without power cuts or injected I/O errors; the
existing [crash](crash-testing.md) and [fault](../crates/afsplus-check/tests/faults.rs)
matrices own those models.

Generated sequences contain operations the core admits, so a refusal is a case
failure. The refusal contract of the version-9 commands belongs to
[the scenario tests](../crates/afsplus-check/tests/scenario.rs): a symlink source
or victim of a replacement, a directory victim, a reservation on a directory, a
policy change on a volume without the feature, a reservation, write or
truncation whose block or record budget is too small, a budgeted truncation of a
directory and a batch member whose parent is a file each become a captured
operation failure at a known index.

Deliberate limits of these oracles: an orphan
holds at most the cleanup budget of extent
records, so multi-step cleanup of a fragmented orphan belongs to
[orphan qualification](orphan-qualification.md); the per-file policy is observed
as a flag, and its effect on write placement belongs to
[data-policy qualification](data-policy-qualification.md). Each verdict compares
the remounted state; intermediate reads and concurrent callers are separate
properties.

## Executable surface audit

Every entry of `ApiMethod` in
[the flight recorder](../crates/afsplus-core/src/flight.rs) names one executable
host surface, and each one belongs to exactly one line below.

| Mutable method | Wire command | Families |
|---|---|---|
| `CleanupOrphan` | `cleanup_orphan` | orphan |
| `CloneFile` | `clone_file` | namespace, captured |
| `CloneRange` | `clone_range` | namespace, captured |
| `CreateDirectory` | `mkdir` | every family |
| `CreateFileInDirectory` | `create` | every family |
| `CreateSymlink` | `symlink` | namespace, captured |
| `DeleteFile` | `unlink`, the `delete` batch member | every family |
| `LinkFile` | `link` | namespace, replace, captured |
| `OrphanFile` | `orphan_file` | orphan |
| `PreallocateFile` | `preallocate` | space, captured |
| `PreallocateFileBounded` | `preallocate_bounded` | space |
| `ReclaimStep` | `reclaim_step` | maintenance |
| `RemoveDirectory` | `rmdir` | every family |
| `Rename` | `rename`, the `rename` batch member | every family |
| `RenameReplace` | `rename_replace`, the `replace` batch member | replace, batch |
| `RenameReplaceOrphanTarget` | `rename_replace_orphan` | orphan |
| `RestoreObjectMetadata` | `restore_metadata` | space |
| `RunBatch` | `batch` | batch |
| `SetFileDataPolicy` | `set_data_policy` | space |
| `SetObjectProtection` | `set_protection` | namespace, space, captured |
| `SnapshotCreate` | `snapshot_create` | snapshot, maintenance, captured |
| `SnapshotDelete` | `snapshot_delete` | snapshot, maintenance, captured |
| `SnapshotMaintenanceStep` | `snapshot_maintenance_step` | maintenance |
| `SnapshotOpen` | `snapshot_open` | snapshot, maintenance, captured |
| `Sync` | `sync` | every family |
| `TruncateFile` | `truncate` | every family |
| `TruncateFileBounded` | `truncate_bounded` | space |
| `UnlinkSymlink` | `unlink_symlink` | namespace, captured |
| `WindowCommit` | `window_commit` | window, batch |
| `WindowFsync` | `window_fsync` | window, batch |
| `WindowOp` | `window_batch` | batch |
| `WindowTruncateFile` | `window_truncate` | window |
| `WindowWriteFileAt` | `window_write` | window |
| `WriteFileAt` | `write` | every family |
| `WriteFileAtBounded` | `write_bounded` | space |

Three mutable methods have no generator of their own.
`CreateDirectoryInRoot`, `CreateFileInRoot` and `DeleteFileInRoot` validate the
timestamp and delegate to `CreateDirectory`, `CreateFileInDirectory` and
`DeleteFile` with the parent fixed to the root object, so a generated sequence
covers their semantics through the delegate. Their own span identity belongs to
[the flight tests](../crates/afsplus-core/tests/flight.rs), and the
[crash](crash-testing.md) and family matrices call them directly.

Five methods set a mount-lifetime budget or policy with no on-disk state.
`SetOrphanCleanupExtentBudget`, `SetSnapshotWorkLimits` and `SetTreeCachePages`
carry the orphan extent budget, the snapshot work limits and the tree cache
profile of the scenario header, so every generated case applies all three at
every mount. The volume-wide `SetDataUpdatePolicy` belongs to
[data-policy qualification](data-policy-qualification.md) and
`SetReclaimBatchBlocks` to the reclaim batch of
[ADR-036](../adr/ADR-036-reclaim-queue.md) and its crate tests;
the persistent per-file policy of [ADR-065](../adr/ADR-065-persistent-data-update-policy.md)
is the generated one.

Four methods serve the security preservation container of
[ADR-101](../adr/ADR-101-security-preservation-container.md):
`SetSecurityDescriptor` and `ClearSecurityDescriptor` mutate,
`SecurityDescriptor` reads and `SetSecurityProjectionPolicy` sets a
mount-lifetime policy. They have no wire command; the
[container test](../crates/afsplus-check/tests/security_container.rs) owns
their proof; no generated family carries descriptors yet.

`SetVolumeLabel` relabels the volume in one commit
([ADR-104](../adr/ADR-104-volume-label-in-checkpoint.md)). It has no wire
command; [the label test](../crates/afsplus-check/tests/volume_label.rs) owns
its proof, power-cut matrix included.

`FileAllocationFrom` reads the committed allocation of a file from a byte
offset in one tree descent. It has no wire command;
[the seek test](../crates/afsplus-core/tests/extent_seek.rs) owns its proof,
including entry-for-entry agreement with `FileAllocationPage` over a
fragmented file.

`SetObjectComment` sets or removes an object's comment in one commit, and
`ObjectComment` and `SnapshotObjectComment` read it
([ADR-106](../adr/ADR-106-stored-object-comment.md)). They have no wire
command; [the comment test](../crates/afsplus-check/tests/object_comment.rs)
owns their proof, power-cut matrix included.

The remaining twenty-three methods read: `FileAllocationPage`,
`FileDataPolicy`, `FirstOrphan`, `ListDirectory`, `ListRoot`,
`LookupInDirectory`, `LookupRoot`, `OrphanCount`, `OrphanObject`,
`QuarantineContains`, `ReadDirectoryPage`, `ReadFile`, `ReadFileAt`, `ReadLink`,
`SnapshotAllocationPage`, `SnapshotList`, `SnapshotLookup`,
`SnapshotReadDirectoryPage`, `SnapshotReadFileAt`, `SnapshotReadLink`,
`SnapshotStat`, `Stat` and `VisibleMetadata`. The observation of every generated
case reads through `ListDirectory`, `Stat`, `ReadFile`, `ReadLink`,
`FileAllocationPage`, `FileDataPolicy`, `OrphanCount` and `OrphanObject`, and a
captured view reads through the five snapshot readers and
`SnapshotAllocationPage`. `ListRoot`, `LookupRoot`, `LookupInDirectory`,
`ReadFileAt`, `ReadDirectoryPage`, `SnapshotList`, `SnapshotLookup`,
`FirstOrphan`, `QuarantineContains` and `VisibleMetadata` are paging and lookup
accessors of [the developer harness](developer-harness.md), whose bounds the
[typed caller properties](#typed-caller-and-unicode-properties) and the crate
tests own.

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
