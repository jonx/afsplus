# Conformance Suite

> **ADRs:** none · **Spec:** none ·
> **Tests:** none · **Milestones:** M00, M01, M02, M05, M12, M14

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
an invalid root LBA so validation cannot stop at the CRC. It corrupts both
slots and requires failure, duplicates a generation across both slots and
requires ambiguity failure, rejects a bad identification checksum, and rejects
an undersized scratch buffer. A feature/root mismatch in the selected newest
checkpoint must fail instead of being hidden by fallback. Structured
diagnostics must identify the failing stage, block and per-slot status. The
gate also runs AddressSanitizer and UndefinedBehaviorSanitizer when supported,
compiles the public example through CMake, and compiles the freestanding reader
for AROS m68k when that toolchain is available.

The same Rust-built fixture contains more than 300 files so both the object map
and root directory require internal nodes. C walks the directory by ordinal,
looks up the selected object through the object map and compares file reads in
777-byte caller buffers with the original input, crossing logical-block
boundaries. A wire-valid extent leaf maps those Rust-produced data blocks after
a sparse hole, exercising extent-floor lookup and zero filling. Checksum and
valid-checksum identity corruption in a selected tree node, plus a
valid-checksum invalid extent flag, and a parent/child subtree-count mismatch
must identify the exact failing LBA and decode phase.

A second Rust fixture leaves a three-record fsynced prefix containing an
existing-file write, truncate and logged create. C must scan and
content-verify the prefix, expose the final size and reproduce both files byte
for byte without changing the image. A damaged replacement block and sequence
gap terminate at their exact tail coordinates; a v3 record without its
required feature bit is a hard format error. Namespace overlay for
rename/delete, non-ASCII comparison-key conformance and exhaustive repair
walking are separate expansion gates.

## Portable C mutation gate

The [fuzzing contract](fuzzing.md#portable-c-corpus-contract) extends the
handwritten corruption matrix with compact recordings of successful Rust-to-C
paths:

```sh
make portable-c-fuzz-gate
```

Six sparse-device packets cover probe, object lookup, both sides of the
multi-leaf directory, a complete directory-to-file read and an intent-log
prefix scan with referenced data. Each receives
4,096 reproducible mutations under ASan/UBSan by default. A failure reports
the seed and stable case number; the included artifact/replay tools reconstruct
its exact bytes and print the terminal stage and LBA. Native libFuzzer consumes
the same entry point and seeds when its runtime exists, but is not required for
the deterministic gate.
