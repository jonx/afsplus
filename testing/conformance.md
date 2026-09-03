# Conformance Suite

> **ADRs:** none · **Spec:** none ·
> **Tests:** [corruption-corpus](corruption-corpus.md) · **Milestones:** M00, M01, M02, M05, M12, M14

A conforming implementation is tested against versioned images.

Required image families:

- empty
- root-only
- nested directories
- long UTF-8 names
- normalization collisions
- case-sensitive collisions
- case-insensitive rejection
- hard links
- symlinks
- sparse file
- multi-extent file
- large file
- region-boundary allocation
- dirty journal
- incomplete transaction
- alternate superblock recovery
- unknown COMPAT feature
- unknown RO_COMPAT feature
- unknown INCOMPAT feature
- stale catalog
- truncated change stream
- metadata checksum corruption

Every image ships with a JSON expectation file.

<!-- toc -->

- [Official tool round trip](#official-tool-round-trip)
- [Checker corruption corpus](#checker-corruption-corpus)
- [Portable C bootstrap gate](#portable-c-bootstrap-gate)
- [Portable C writer gate](#portable-c-writer-gate)
- [Portable C fuzz mutation gate](#portable-c-fuzz-mutation-gate)

<!-- /toc -->

## Official tool round trip

The official formatter and inspectors have an executable host-side contract:

```sh
cargo test -p afsplus-tools --all-features
```

Black-box tests invoke the installed command names rather than private parsing
functions. Fixed UUID and timestamp inputs make formatter JSON exact and
repeatable. All five compatibility profiles must format and cross-read. A
populated image verifies deterministic object, directory and extent output.
After the image is changed to host mode `0444`, repeated info and dump runs
must leave every byte and the permission mode unchanged. Corrupt media exits
1 with a surface-specific diagnostic ID; bad invocation and missing host files
exit 2. Help exits 0.

This gate proves the CLI and schema contract. The Rust/C cross-reader fixtures
below separately prove that the formatter's bytes are portable rather than a
Rust-only interpretation.

## Checker corruption corpus

The [dedicated corruption corpus](corruption-corpus.md) generates twelve
small images from one fixed Rust-formatted reference volume. Identification,
checkpoint, typed-tree, object-record, allocation-bitmap and intent-log
surfaces each receive a checksum failure and a valid-checksum semantic
failure. Every case records its exact byte mutation and expected structured
checker result in a versioned manifest, and every exported artifact must be
byte-reproducible across runs.

## Portable C bootstrap gate

The independent C99 slice validates the immutable identification block and
structurally selects the newest checkpoint using caller-provided block I/O and
one caller-provided scratch block. It is compiled with strict warnings and
reads an image formatted and advanced through multiple generations by the Rust
implementation:

```sh
make portable-c-gate
```

The executable corrupts the newest checkpoint and requires fallback to the
retained older generation, then repeats that check with a valid checksum over
an invalid root LBA so validation cannot stop at the CRC. Valid-CRC mutations
of checkpoint header flags, header owner and payload flags each require the
same fallback; placing the same mutation in both slots requires failure with
both per-slot statuses marked corrupt. It also duplicates a generation across
both slots and requires ambiguity failure, rejects a bad identification
checksum, and rejects an undersized scratch buffer. A feature/root mismatch in
the selected newest checkpoint must fail instead of being hidden by fallback.
Structured diagnostics must identify the failing stage, block and per-slot
status. The gate also runs AddressSanitizer and UndefinedBehaviorSanitizer when
supported, compiles the public example through CMake, and compiles the
freestanding reader for AROS m68k when that toolchain is available.

The same Rust-built fixture contains more than 300 files so both the object map
and root directory require internal nodes. C walks the directory by ordinal,
looks up the selected object through the object map and compares file reads in
777-byte caller buffers with the original input, crossing logical-block
boundaries. A wire-valid extent leaf maps those Rust-produced data blocks after
a sparse hole, exercising extent-floor lookup and zero filling. Checksum and
valid-checksum identity corruption in a selected tree node, plus a
valid-checksum invalid extent flag, and a parent/child subtree-count mismatch
must identify the exact failing LBA and decode phase.

A second Rust fixture leaves a seven-record fsynced prefix containing
hard-link deletion, an existing-file write and truncate, logged create, rename
chains and replacement. C must scan and content-verify the prefix, resolve and
enumerate its final case-insensitive namespace, preserve surviving object
identity, reject the replaced object, and reproduce both final files byte for
byte without changing the image. Valid-checksum semantic corruptions cover a
missing rename source, replacement-flag mismatch, create-ID regression and
write-size mismatch. A damaged replacement block and sequence gap terminate at
their exact tail coordinates; a v3 record without its required feature bit is
a hard format error. Non-ASCII Unicode comparison-key conformance and
exhaustive repair walking are separate expansion gates.

## Portable C writer gate

The same gate compiles a separate ABI-1 writer and gives it copies of the
seven-record Rust fixture. `afspw_create_empty_file`,
`afspw_rename_file_no_replace`, `afspw_delete_file` and
`afspw_rename_file_replace` must each append sequence 8 using exactly one
block write and one flush. The C reader observes each resulting durable
namespace; Rust then replays the C-produced record, verifies the empty create's
monotone ID and zero bytes, verifies visible contents, confirms final victims
retain byte-exact orphan contents and runs the exhaustive checker.

A 64-byte torn record returns write-uncertain and is safely overwritten after
a fresh scan. A failed flush returns durability-uncertain without claiming
success. An existing target and a full log return before media I/O. Delete and
replacement are emitted only when the volume carries ADR-066's orphan feature.
Create additionally rejects a case-folded collision, a missing parent and an
exhausted object-ID space before media I/O, with a create-specific diagnostic
stage. A valid-checksum checkpoint with a regressed allocator watermark is
rejected against the greatest committed object by a bounded tree lookup. Both
the 144-read 8 KiB create and 21-read cached create are replayed in Rust.
The writer is included in strict C99, ASan/UBSan, static-analysis, CMake
install/consumer and configured m68k compile gates. The 8 KiB minimum-memory
path and the recommended 56 KiB call-local-cache path are both replayed by
Rust. On the fixed seven-record fixture their preflight ceilings are 152 and
21 reads respectively; a torn-write retry is at most 42 cached reads across
both attempts. The configured m68k compiler must keep the main writer frame at
or below 1 KiB. A failed first log read must identify the intent-scan stage
and exact LBA, perform no write or flush, and succeed on a fresh 25-read
retry. This qualifies four bounded namespace operations, not allocation or a
complete classic-rw profile.

## Portable C fuzz mutation gate

The [fuzzing contract](fuzzing.md#portable-c-corpus-contract) extends the
handwritten corruption matrix with compact recordings of successful Rust-to-C
paths:

```sh
make portable-c-fuzz-gate
```

Seven sparse-device packets cover probe, object lookup, both sides of the
multi-leaf directory, a complete directory-to-file read, an intent-log prefix
scan with referenced data and a final durable namespace lookup. Each receives
4,096 reproducible mutations under ASan/UBSan by default. A failure reports
the seed and stable case number; the included artifact/replay tools reconstruct
its exact bytes and print the terminal stage and LBA. Native libFuzzer consumes
the same entry point and seeds when its runtime exists, but is not required for
the deterministic gate.
