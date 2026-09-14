# ADR-029: Rust primary implementation plus portable C implementation

Status: Accepted

## Context

AFS+ aims to be adopted beyond one AROS target. The reference implementation therefore has two conflicting needs:

- move quickly with strong safety, testing, and tooling
- remain implementable on systems without a modern Rust toolchain

Rust is a good fit for the primary implementation, but making Rust mandatory for every consumer would work against portability across classic and niche operating systems.

## Decision

AFS+ will maintain two implementation tracks against the same normative on-disk specification and conformance suite.

### Primary modern implementation

Rust is the primary implementation language for:

- `afsplus-core`
- host tools
- FUSE integration
- fuzzing/property tests
- deterministic fault injection
- AROS modern integration where practical

### Portable C implementation

A portable C implementation is a first-class project deliverable, not a temporary bootstrap.

It must target environments with:

- C99 or a clearly documented smaller compatibility subset where necessary
- no POSIX requirement in the core
- caller-provided allocation hooks
- caller-provided block I/O
- no mandatory threads
- bounded-memory operation
- explicit endian conversion

The C implementation may expose profiles:

- `reader-minimal`
- `classic-rw`
- `full-portable`

Not every optional accelerator must be implemented by every profile.

## Single source of truth

The Rust implementation is not the specification.

The normative sources are:

- on-disk format specification
- invariants
- feature registry
- compatibility rules
- conformance images
- semantic operation traces

Both implementations must be tested independently against those artifacts.

## Cross-implementation validation

CI must continuously test interoperability in both directions:

```text
Rust mkfs -> C read/check
C mkfs    -> Rust read/check
Rust write -> C read/write/check -> Rust verify
C write    -> Rust read/write/check -> C verify
```

For deterministic workloads, semantic final state and invariant results must match even if physical allocation decisions legitimately differ.

## Performance policy

Neither implementation is assumed to be faster.

CPU, memory, I/O, latency, and write amplification are measured using the benchmark specification. Regressions are tracked per implementation and per profile.

The portable C implementation is allowed to be simpler than the Rust implementation where optional features are absent, but baseline correctness and disk compatibility are identical.

## Consequences

- AFS+ remains attractive to AROS, Amiga-family systems, recovery tools, boot environments, and independent ports.
- Rust can optimize development speed without becoming an ecosystem dependency.
- Having two independent implementations substantially increases the chance of detecting specification ambiguities and correlated implementation bugs before format freeze.
