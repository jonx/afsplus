# Embedding the portable C reader and bounded writer

The reader is a small C99 library for boot, recovery, classic and third-party
integrations. It shares no executable code with the Rust implementation.

<!-- toc -->

- [Integration choices](#integration-choices)
- [ABI and capability policy](#abi-and-capability-policy)
- [Runtime contract](#runtime-contract)
- [Durable intent-log view](#durable-intent-log-view)
- [Bounded classic-rw mutations](#bounded-classic-rw-mutations)
- [Validation](#validation)
- [Fuzzing and exact reproduction](#fuzzing-and-exact-reproduction)

<!-- /toc -->

## Integration choices

For source vendoring, compile [`reader.c`](reader.c), plus
[`writer.c`](writer.c) when mutation is needed, and add
[`../../api`](../../api) plus [`../../spec`](../../spec) to the private include
path. Consumers include only
[`libafsplus_reader.h`](../../api/libafsplus_reader.h) and optionally
[`libafsplus_writer.h`](../../api/libafsplus_writer.h); the wire-format header
is an implementation dependency.

For CMake:

```sh
cmake -S portable/c -B build/portable-c
cmake --build build/portable-c
cmake --install build/portable-c --prefix /chosen/prefix
```

An enclosing CMake project may use `add_subdirectory(portable/c)` and link
`AFSPlus::reader` or `AFSPlus::writer` (which carries the reader dependency).
Set `AFSPLUS_READER_BUILD_EXAMPLE=OFF` when embedding only the libraries. An
installed package supports:

```cmake
find_package(AFSPlusReader 0.1 CONFIG REQUIRED)
target_link_libraries(your_target PRIVATE AFSPlus::reader)
# or: target_link_libraries(your_target PRIVATE AFSPlus::writer)
```

## ABI and capability policy

This additive slice keeps separate reader ABI 1 and writer ABI 1 surfaces and
packages as version 0.1. Existing
probe calls and the original placeholder `struct afspr_entry` keep their
layout; the operational iterator uses the separate
`struct afspr_directory_entry`. Every output struct carries the ABI version,
and calls that write structs also receive the caller's size. New code checks
`afspr_capabilities()` instead of inferring support from a package version.
None of these reader calls changes Filesystem API v2 or an application-facing
filesystem-neutral capability.

The writer header has its own capability mask, result and diagnostic structs.
Its writer-only status values do not renumber the reader ABI. Adding a future
operation does not make an integration infer support from the package version.

## Runtime contract

The caller provides:

- a logical-block callback returning zero only after it filled the complete
  requested range;
- the actual block count and logical block size of that device view;
- one writable scratch buffer of at least one logical block,
  `AFSPR_TREE_SCRATCH_SIZE` bytes for tree iteration, or
  `AFSPR_INTENT_SCRATCH_SIZE` bytes for intent-log operations; and
- storage for the result and, optionally, structured diagnostics.

The bootstrap implementation accepts the prototype's 4-KiB logical blocks.
It performs exactly three successful block reads for a valid probe: the
identification block and checkpoint slots A/B. It allocates nothing, writes
nothing, retains no caller pointer, has no mutable global state and may be
used concurrently when calls use distinct callbacks and scratch buffers.

`afspr_capabilities()` lets an integrator test the additive reader surface.
After `afspr_probe_detailed`, `afspr_lookup_object` performs a bounded object
map lookup, `afspr_directory_entry_at` reads one entry by tree ordinal, and
`afspr_read_file` fills a caller-owned bounded buffer. Direct extents, extent
trees, sparse holes and unwritten mappings are understood. Returned directory
names live in the caller's name buffer. Passing zero name capacity is a sizing
query: `AFSPR_ERR_BUFFER_TOO_SMALL` leaves the required length in
`entry.name_len`.

The object decoder recognizes the per-file private-in-place policy flag from
[ADR-065](../../adr/ADR-065-persistent-data-update-policy.md) without changing
read semantics. A flagged directory, an unknown object flag, or a flagged file
on a volume without `AFSPR_COMPAT_DATA_POLICY` is corruption rather than an
ignored policy.

Object admission is exact, as in the Rust decoder: zero common-header flags,
a payload of exactly 96 bytes for a file or a directory, 112 bytes when the
record carries `AFSPR_OBJECT_FLAG_SECURITY_REF`, the inline target after
either length for a symlink, and a zero tail. The 16-byte security reference
is validated (nonzero first segment, length of 1 to 65,536 bytes, the segment
count that length implies, assigned reference flags) and, on the volume path,
requires `AFSP_INCOMPAT_SECURITY_DESCRIPTORS` and an allocatable first
segment. `afspr_decode_security_reference` and
`afspr_decode_security_segment` are standalone, heap-free decoders for the
reference of a record and for one `"AFSX"` segment. The comment of
[ADR-106](../../adr/ADR-106-stored-object-comment.md) follows the reference:
the shared shape check validates its length byte, its NUL-free UTF-8 and the
exact payload length it implies, `afspr_decode_object_comment` returns it, and
an unassigned object flag bit refuses the record in every decoder, the
standalone ones included. The attribute reference of
[ADR-108](../../adr/ADR-108-extended-attributes.md) sits between the security
reference and the comment: the shape check validates its nonzero first block,
its length of 1 to 65,536 bytes, the segment count that length implies and its
zero reserved field. `afspr_decode_attribute_reference` returns it,
`afspr_decode_attribute_segment` decodes one `"AFSA"` segment with the code of
the `"AFSX"` decoder under its own block type and bound,
`afspr_validate_attribute_set` admits a whole set under the exact rules of the
Rust codec, and `afspr_attribute_set_next` steps through an admitted set
without copying. The reader has no volume-level attribute read: a caller walks
the chain with the segment decoder into its own buffer.
[`attributes_c.rs`](../../crates/afsplus-format/tests/attributes_c.rs) holds
the per-image agreement for these four. The reclaim queue of
[ADR-036](../../adr/ADR-036-reclaim-queue.md) has three standalone decoders,
`afspr_decode_reclaim_root`, `afspr_decode_reclaim_segment` and
`afspr_decode_reclaim_table`, with `afspr_reclaim_ref_at` and
`afspr_reclaim_entry_at` to read the areas they validated; the reader does not
walk the queue, since no read path needs it.
[`reclaim_c.rs`](../../crates/afsplus-format/tests/reclaim_c.rs) holds their
per-image agreement with the Rust codec. `afspr_decode_checkpoint_block`
decodes one checkpoint slot against a volume UUID in both payload lengths, 168
bytes and 184 with the snapshot roots, and refuses a nonzero byte after the
payload ([ADR-111](../../adr/ADR-111-checkpoint-zero-tail.md)); the volume
paths sit on top of it and still refuse a checkpoint with snapshot roots.
[`checkpoint_c.rs`](../../crates/afsplus-format/tests/checkpoint_c.rs) holds
its per-image agreement. The reader preserves
these bytes and never evaluates them; the writer appends intent records only
and rewrites no object record, so it cannot drop a reference.
[`security_c.rs`](../../crates/afsplus-format/tests/security_c.rs) holds the
per-image agreement of the C codec, the Rust codec and a literal expectation,
and [the volume test](../../crates/afsplus-check/tests/security_c.rs) the same
agreement on objects of real images.

Every reader operation is read-only, allocation-free and bounded by the
tree-height or configured log-slot cap. The data destination and scratch
buffer must not overlap. `AFSPR_CAP_FILE_READ` names the selected checkpoint
view only; callers explicitly select the durable overlay through the separate
intent capabilities.

`afspr_probe` is the compact call. `afspr_probe_detailed` additionally reports
the failed stage, LBA, checkpoint slot and independent status of both slots.
The result is complete only on `AFSPR_OK`; diagnostics are valid whenever the
caller supplies a full `struct afspr_diagnostic`. Coordinates that do not apply
are reported as `AFSPR_NO_BLOCK` and `AFSPR_NO_CHECKPOINT_SLOT`, so log output
does not confuse a missing coordinate with a real block number.

The callback is the only platform-specific piece. The
[`probe_file.c`](examples/probe_file.c) example shows a host-file adapter and
prints a diagnostic suitable for bug reports. Native AROS integrations replace
that callback with their bounded device viewport rather than adding host I/O to
the reader.

The tree walker validates CRCs, type/owner identity, committed generation,
level transitions, separator bounds, key ordering, subtree counts, child LBAs
and cycles along the selected path. Object records and selected directory or
extent values receive typed validation. ASCII directory names additionally
cross-check their stored comparison key. Full semantic checking of non-ASCII
keys awaits the frozen Unicode 16 tables; exhaustive whole-tree and allocation
ownership checks remain the repair tool's job rather than ordinary bounded
reads.

## Durable intent-log view

`afspr_scan_intent_log` validates the sequence bound to the selected
checkpoint, verifies every referenced replacement block and returns the exact
valid prefix plus the first excluded tail slot and LBA. A stale, empty,
out-of-sequence or content-damaged tail is reported in `tail_state`; it does
not erase earlier complete records. Device I/O and feature-contract failures
remain hard errors with a structured stage and LBA.

`afspr_lookup_intent_directory_entry` resolves a name against the selected
checkpoint plus the durable prefix. `afspr_intent_directory_next` merges the
committed tree and logged create/rename targets in comparison-key order through
a caller-owned cursor. Deletes, rename chains, case-only renames, hard-link
identity and replacement are reflected. A too-small name buffer leaves the
selected entry pending, so retrying the same cursor cannot skip it.

`afspr_intent_file_size` and `afspr_read_intent_file` expose the corresponding
durable file bytes without writing the device or allocating memory. They
handle files created only in the log, sparse growth, partial-block replacement,
shrink, last-link deletion and identity retained through rename or another hard
link. Each operation validates namespace replay preconditions, including
source and target existence, replacement intent, monotone create IDs and
write/truncate expected sizes.

The caller retains the `afspr_intent_view` and passes it back on each lookup,
cursor step or file read. Every call rescans the bounded log and revalidates
the semantic prefix before using the view; no mutable validation cache or
borrowed media pointer crosses calls. This favors a simple anti-TOCTOU contract
over throughput: work is bounded by the configured log slots and operations,
but an integration performing large directory scans should cache results above
the reader only while it can guarantee an immutable device view.

Legacy identity-key volumes support every valid UTF-8 spelling byte-for-byte.
The versioned Unicode profiles support exact ASCII normalization and ASCII
case folding. A non-ASCII logged name on those profiles clears the advertised
intent views, and a direct non-ASCII lookup returns `AFSPR_ERR_UNSUPPORTED`;
neither is approximated with locale rules. Committed ordinal enumeration keeps
the base reader's structural behavior, whose stored non-ASCII comparison keys
cannot receive a semantic name/key cross-check until frozen Unicode 16 tables
are present.

## Bounded classic-rw mutations

`afspw_create_empty_file`, `afspw_rename_file_no_replace`,
`afspw_delete_file`, `afspw_rename_file_replace`, `afspw_truncate_file` and
`afspw_write_file_block_cow` are the independent media-mutating C slice. Each
freshly probes the volume, rescans and semantically validates the durable view
and proves the relevant operands before writing. Namespace calls use version
2. Truncate and COW write use version 3 and require its incompatible feature.
Empty create derives its returned object ID from the complete validated
prefix, including gaps left by prior logged creates.

Namespace and data-free truncate allocate no disk block. They write one record
into the next preallocated intent slot and invoke one flush. The COW call
accepts one caller-assembled complete logical block, validates the committed
allocation-root/descriptor/bitmap chain and all data reservations in the
durable prefix, then chooses one base-checkpoint-free block outside those
reservations while preserving the Rust runtime emergency floor. It writes and
flushes that data before writing and flushing its version-3 record. The bitmap
intentionally stays unchanged until Rust or another full engine replays the
record and preclaims the named block.

The caller supplies read/write/flush callbacks, exclusive writer
serialization and immutable name/data buffers. COW input cannot overlap the
scratch area, growth must end in the replaced logical block, and bytes beyond
a partial final EOF must be zero. The hard minimum remains 8 KiB. Extra
complete 4 KiB blocks in the same scratch area become a call-local read cache,
up to sixteen entries;
`AFSPW_RECOMMENDED_SCRATCH_SIZE` supplies twelve entries (56 KiB total).
Nothing is retained after the function returns, so retry and uncertain-I/O
rules do not depend on cache invalidation.

A torn record is the invalid tail and the same slot can be retried after a
fresh probe. Write or flush callback failure returns an explicitly uncertain
status; the caller must discard cached state and recover/probe before deciding
whether to retry. A full log returns `AFSPW_ERR_LOG_FULL` without write or
flush and requires a checkpoint-capable implementation to materialize the
prefix.

The COW path distinguishes data-write, data-flush, record-write and
record-flush uncertainty. A failed or torn data write has no durable reference
and can be overwritten after a fresh probe. A failed record write or flush may
have made the new extent reachable, so retry begins with recovery/probe rather
than assuming either outcome. Allocation failures report the metadata or data
LBA and preserve zero write/flush behavior; exhaustion returns
`AFSPW_ERR_NO_SPACE` before changing media.

`afspw_truncate_file` supports sparse growth and block-aligned shrink. A
same-size request is a successful zero-I/O no-op reported explicitly in its
result. An unaligned shrink fails with
`AFSPW_ERR_TAIL_REWRITE_REQUIRED` before media I/O: conservatively rewriting
the retained partial block needs the later COW data-allocation slice, even
when a particular layout might prove that the tail is already sparse.

Delete and replacement require the ADR-066 orphan-directory feature. Rust
replay preclaims logged data, then moves each final victim into persistent
orphan state in the same checkpoint, lazily creating object 2 there if needed;
it never retires the victim layout in proportion to fragmentation. Directory
rename, nonempty create, arbitrary byte-range or multi-block writes, unaligned
shrinking truncate, bitmap/checkpoint publication, orphan maintenance and
checkpoint materialization are later `classic-rw` slices. Modern Unicode
profiles accept ASCII lookup names in this C path; legacy identity volumes
retain exact UTF-8 lookup.

## Validation

From the repository root:

```sh
make portable-c-gate
```

The gate cross-reads a Rust image with more than 300 objects, forcing both its
object map and root directory beyond one leaf. It enumerates to a real file,
decodes its object and compares bounded C reads with the original bytes. A
synthetic but wire-valid extent root reuses those Rust-produced data blocks to
exercise a sparse hole and typed extent lookup. Valid-checksum tree identity
and extent-value corruptions must report the exact failing LBA.

A second Rust fixture leaves seven fsynced records beyond its checkpoint:
hard-link deletion, existing-file write and truncate, create, a two-step rename
chain, another rename and replacement. C scans that prefix, resolves the final
case-insensitive names, enumerates them in key order and reconstructs both
surviving files in 777-byte reads against Rust-written oracle bytes. It also
requires exact record coordinates for missing rename sources, replacement
flag mismatch, non-monotone create IDs and write-size mismatch. Content damage,
a sequence gap and a missing v3 feature bit stop or fail at the exact record or
data LBA. The replacement victim carries Rust's persistent private-in-place
flag on a data-policy volume, while the primary fixture injects the same flag
without its feature and requires object-decode failure.

The C writer copies that seven-record image and independently appends an eighth
empty create, one-block COW write, data-free truncate, non-replacing rename,
delete and replacing rename. Metadata-only mutations use one write and one
flush; COW write uses data-write/data-flush then record-write/record-flush. The
C overlay verifies the result, then Rust replay checks the exact replacement
bytes, monotone create ID, zero shrink, sparse growth, visible content,
retained orphan bytes and all filesystem invariants.
Case-insensitive create collision, a missing parent and exhausted object IDs
all produce exact diagnostics and zero writes. A syntactically valid
checkpoint whose object-ID watermark regresses below the highest object is
also rejected on a bounded right-edge tree lookup rather than being amplified
by the writer. Same-size truncate performs no I/O; missing data-log support and
an unaligned shrinking tail fail explicitly before I/O, as do missing or
non-file object IDs with file-state diagnostics. A 64-byte torn
namespace or truncate record is retried into the same slot; flush failure
reports durability uncertainty; an existing destination and a full log
produce zero media writes.
The COW matrix covers a torn data write, failed data flush, torn record, failed
record flush, exact descriptor/bitmap read diagnostics, nonzero EOF tail,
invalid shrinking range and a 512-block fixture whose log reserves every
ordinary-growth block while leaving the 16-block emergency floor untouched.
Every refused case leaves authoritative media unchanged.
Strict warnings, ASan/UBSan, Clang static analysis, CMake export consumption
and the configured m68k compiler include both reader and writer. At the
seven-record prefix, the 8 KiB namespace fallback makes at most 152
logical-block reads; truncate needs at most 178 and COW allocation needs at
most 223. The recommended twelve-entry cache makes at most 21 for every
metadata-only mutation, while a torn-write retry makes at most 42 across both
complete preflights. Cached COW write uses at most 25 reads, or 50 across a
data/record torn retry. Empty create's 8 KiB path is independently capped at
144 reads. The test fixes these as
non-regression ceilings and cross-replays both memory profiles in Rust. The
configured m68k compiler must also keep every writer function frame at or
below 1 KiB (the largest measured frame is 768 bytes). The read counts are
structural, not device-latency claims; merging the file-state and namespace
passes remains a possible later low-memory optimization.
The matrix also injects an I/O failure on the first log read, requires the
exact writer stage and LBA with zero writes, then retries successfully in 25
total reads. For a block-by-block preflight trace while diagnosing an adapter,
run `AFSPLUS_TRACE_WRITER_READS=1 make portable-c-gate`; the test harness emits
the mode, LBA and block count without changing library behavior.

The same gate tests retained-checkpoint fallback and corrupt/ambiguous states.
It independently sets the checkpoint header flags, header owner and payload
flags on Rust-produced blocks, reseals their CRCs, and requires both fallback
when one slot is invalid and complete selection failure when both slots are
invalid. This pins the C reader to the Rust decoder's reserved-field contract
instead of merely checking torn writes. The gate also compiles the public
example and runs AddressSanitizer/UndefinedBehaviorSanitizer plus the
compiler's static analyzer when supported. If the local AROS m68k compiler
exists, it compiles the library for that target without linking host
facilities. A separate test project consumes the CMake install, so a broken
exported target cannot pass.

## Fuzzing and exact reproduction

Run the bounded mutation gate separately with:

```sh
make portable-c-fuzz-gate
AFSPLUS_FUZZ_RUNS=250000 make portable-c-fuzz-long
```

The seed packer records only blocks actually read during a successful
operation. Its `.afzf` packet carries the virtual device size, operation and
arguments followed by `(LBA, 4096-byte block)` records. This keeps a complete
probe, lookup, directory descent, file read or durable namespace lookup between
12 and 48 KiB instead of copying a 16-MiB image. The packet reader treats
absent blocks as I/O failures and never writes the source packet.

The mandatory smoke engine makes every mutation a pure function of a seed and
case number. It mutates every packet-header byte and the first 128 bytes of
every observed block twice: once as a torn checksum and once with a recomputed
CRC so structural decoders are reached. Later cases mix block-wide flips,
overwrites, multi-byte changes and truncations, resealing alternate cases. If
a sanitizer stops the process, the gate prints a progress file containing the
seed and case. Recreate and preserve the exact input with:

```sh
build/afsplus-fuzz-smoke --case 1234 --artifact failure.afzf seed.afzf
build/afsplus-fuzz-replay failure.afzf
```

The replay line includes each invoked operation's symbolic result plus the
last diagnostic stage and LBA. This makes a small, immutable regression case
that any developer can run. CMake can build `afsplus-fuzz-pack`,
`afsplus-fuzz-replay` and
`afsplus-fuzz-smoke` with `-DAFSPLUS_READER_BUILD_FUZZ_TOOLS=ON`.

[`fuzz_target.c`](fuzz/fuzz_target.c) also exports the standard
`LLVMFuzzerTestOneInput` entry point. When `-fsanitize=fuzzer` is available,
the repository gate invokes it on the same corpus. Some Apple command-line
tool installations omit the libFuzzer runtime; that is reported as a skip,
while the deterministic ASan/UBSan engine remains mandatory.
