# Orphan Lifecycle Qualification

> **ADRs:** [ADR-066](../adr/ADR-066-bounded-orphan-directory.md) · **Spec:**
> [feature registry](../spec/feature-registry.toml) · **Tests:**
> [crash testing](crash-testing.md) · **Milestones:** M03, M05, M14

This plan qualifies [ADR-066](../adr/ADR-066-bounded-orphan-directory.md)
without treating a successful mount as proof of crash consistency.

## Gates

```sh
cargo test -p afsplus-vfs --test api
cargo test -p afsplus-check --test orphans
make portable-c-gate
```

The VFS matrix covers multiple handles, read/write/truncate/fsync after final
unlink, hard links, immediate name reuse, guessed-ID hiding, last-close
cleanup, open-target atomic replacement and the explicit legacy-volume
fallback when the feature is absent. It also fills a multi-node allocation
geometry with a deep namespace and fragmented file, proves no-handle unlink
uses bounded orphan state near ENOSPC, then drains it with a deliberately tiny
general reclaim setting.

The checker matrix covers absent, empty and populated object-2 states. It
forges valid-checksum semantic damage for a missing feature bit, noncanonical
orphan name, wrong child type, duplicate visible reference and a public link
to object 2. Every case must fail with a localized diagnostic.

## Crash oracle

The power-cut harness enumerates every recorded write/flush boundary plus its
modeled unflushed-tail subsets and representative torn writes for:

- lazy object-2 creation and first orphan insertion;
- a data update after unlink;
- each tail cleanup and final object-removal checkpoint;
- atomic replacement while the old target remains open.

Each recovered image must pass the exhaustive checker and match only the
semantic states listed in [crash-testing.md](crash-testing.md). The test never
accepts “mount succeeded” on its own.

## Resource and progress contract

A fragmented orphan is constructed with more logical extents than the runtime
budget. One maintenance call must remove no more than that budget from the
logical tail. Remount must resume from the selected checkpoint without an
external cursor, and repeated calls must eventually remove the object. Each
intermediate image must remain checker-clean.

Qualification output records, per cleanup step, extent records removed,
remaining allocated bytes, block reads/writes, bytes written, flushes,
allocator bitmap RAM and the configured extent budget. Q3 low-space
qualification separately enforces and reports the emergency metadata
headroom required for the bounded namespace and cleanup transactions. Orphan
cleanup has a 16-block minimum reclaim-promotion budget so repeated small
steps cannot lose forward progress to their own COW metadata.

## Portable-reader contract

The C99 reader must report the `RO_COMPAT` feature, return `NOT_FOUND` for
ordinary lookup of object 2, stay warning-free under strict/sanitized/static
analysis builds and compile with the configured m68k compiler. Diagnostic dump
output, unlike ordinary lookup, labels object 2 and reports its entry count.
