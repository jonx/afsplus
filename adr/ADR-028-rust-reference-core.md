# ADR-028: Rust reference core with language-neutral format

Status: Accepted implementation direction

## Context

AFS+ must be portable across modern AROS, host development systems, recovery tools, and potentially classic Amiga-family systems.

Rust offers strong advantages for new filesystem code: memory safety for ordinary references and collections, checked abstractions around block parsing, enums for explicit state machines, powerful property/fuzz testing, and a good fit for deterministic host-side tooling.

At the same time, requiring Rust for every implementation would unnecessarily restrict bootloaders, classic systems, independent implementations, and low-level recovery environments.

## Decision

The primary modern reference implementation of AFS+ will be developed in Rust.

The on-disk format, invariants, feature registry, and management protocol remain language-neutral specifications.

The implementation is split conceptually into:

```text
afsplus-core
    no OS namespace assumptions
    block-device abstraction
    parsing/encoding
    allocator
    trees/extents
    transactions/checkpoints
    checker/invariants

host adapters
    file image backend
    memory backend
    FUSE adapter
    tools

AROS adapter
    block-device backend
    DOS/filesystem handler glue
    Filesystem API v2 glue
```

## `no_std` boundary

The core should be designed so that foundational crates can build with `#![no_std]`, using `core` and, where required, `alloc` behind an explicit allocator contract.

Host tools may use normal Rust `std`.

This makes the core suitable for constrained or kernel-like environments without forcing all tools into `no_std`.

## C ABI

A stable C-facing ABI is required for integration and independent tooling.

AROS does not need every filesystem-facing component rewritten in Rust immediately. A thin C handler may call a Rust static library, or the handler may progressively move to Rust as AROS Rust support matures.

## Classic and minimal implementations

A tiny portable C reader remains a project requirement for boot, recovery, and classic-system use.

Rust is therefore the reference implementation language, not part of the disk-format compatibility contract.

## Consequences

- AFS+ can be developed and tested on macOS/Linux before an AROS handler exists.
- The same core can later run inside AROS without changing disk semantics.
- AROS Rust driver support can evolve independently.
- Memory-unsafe FFI boundaries are small and auditable.
- Independent non-Rust implementations remain possible.
