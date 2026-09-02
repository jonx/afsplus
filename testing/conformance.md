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

Object-tree traversal, intent-log inspection and file reads are separate
expansion gates.
