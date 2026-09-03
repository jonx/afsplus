# Fuzzing

> **ADRs:** none · **Spec:** none ·
> **Tests:** [`check-portable-c-fuzz.sh`](../tools/check-portable-c-fuzz.sh) · **Milestones:** M01

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

| Wire surface | Portable C path corpus | Remaining work |
|---|---|---|
| Identification and retained checkpoints | Probe seed, header/block mutations | Add frozen reserved-field decisions |
| Object record and object-map node | Root lookup plus multi-leaf paths | Add every object type and optional root |
| Directory node | First/last ordinal in a 303-entry tree | Add complete Unicode comparison-key tables |
| Extent node and file data | Directory-to-file read seed; direct/sparse synthetic coverage remains in conformance | Add committed tree-backed file seed |
| Allocation-region metadata | None | Add with portable repair walker |
| Intent-log record | None | Add with C intent replay |
| Xattr record | None | Add when the portable reader exposes xattrs |
| Catalog record | None | Add with catalog implementation |
| Change-stream record | None | Add with change-stream implementation |

## Portable C corpus contract

`make portable-c-fuzz-gate` builds a Rust image with 303 root entries and
records five successful operations: probe, root-object lookup, both extremes
of a multi-leaf directory, and directory-to-file lookup/read. The recorder
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
