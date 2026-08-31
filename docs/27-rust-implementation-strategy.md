# 27. Rust Implementation Strategy

> **ADRs:** [ADR-042](../adr/ADR-042-aros-c-boundary.md),
> [ADR-043](../adr/ADR-043-native-aros-dospacket-translator.md) · **Spec:** none ·
> **Tests:** none · **Milestones:** none

<!-- toc -->

- [1. Why Rust is attractive for AFS+](#1-why-rust-is-attractive-for-afs)
- [2. Rust does not mean waiting for native Rust on AROS](#2-rust-does-not-mean-waiting-for-native-rust-on-aros)
- [3. Proposed crate layout](#3-proposed-crate-layout)
- [4. Block-device trait](#4-block-device-trait)
- [5. Composable test backends](#5-composable-test-backends)
- [6. Rust on Macaros Native](#6-rust-on-macaros-native)
- [7. What should remain C-accessible](#7-what-should-remain-c-accessible)
- [8. Testing advantage](#8-testing-advantage)
- [9. Development principle](#9-development-principle)

<!-- /toc -->

## 1. Why Rust is attractive for AFS+

Filesystem code is unusually exposed to memory corruption, stale references, integer overflow, malformed input, lifetime mistakes, and complex state transitions.

Rust does not make filesystem logic automatically correct, but it removes or constrains several bug classes that are especially expensive in storage code:

- use-after-free and many stale pointer bugs
- accidental aliasing of mutable state
- unchecked enum/state variants
- accidental buffer overrun in ordinary safe code
- ad-hoc ownership of cache pages and transaction objects

It also makes it natural to model on-disk parsing as checked transformations from bytes into validated typed records rather than casting raw blocks directly to mutable C structs.

## 2. Rust does not mean waiting for native Rust on AROS

AFS+ development should begin on a normal host before Macaros Native can mount it.

Initial environment:

```text
macOS / Linux
     |
Rust host tools
     |
afsplus-core
     |
FileBlockDevice("test.afsplus")
```

The file contains the same raw AFS+ block layout that a physical partition will contain later.

Therefore the allocator, directory tree, extent tree, transaction/checkpoint engine, checker, reflinks, catalog, change stream, and crash tests can all be developed independently of Apple-Silicon hardware drivers.

## 3. Proposed crate layout

```text
crates/
  afsplus-core/          # no_std-capable core format/algorithm code
  afsplus-format/        # exact encoding, constants, checksums
  afsplus-check/         # invariant/check engine
  afsplus-block/         # block-device traits and wrappers
  afsplus-vfs/           # portable handles, errors, capabilities, I/O API
  afsplus-host/          # std host adapters
  afsplus-fuse/          # host filesystem mount adapter
  afsplus-cli/           # mkfs/info/check/trace/explain tools
  afsplus-aros/          # safe, packet-neutral DOS handler semantics
  afsplus-aros-ffi/      # versioned staticlib/C callback boundary
```

The exact crate split can change as implementation begins. The important boundary is that core disk semantics do not depend on POSIX, macOS, AROS DOS packets, FUSE, or a host filesystem namespace.

The `afsplus-vfs` boundary is now executable. Both `afsplus-fuse` and
`afsplus-aros` consume it; neither adapter may reach into COW trees or disk
records directly. `afsplus-aros-ffi` confines all raw pointers and native
block-device callbacks around that safe adapter. The C translator in
`native/aros` owns only AROS packet structures, native wrappers and DOS path
walking; see ADR-042 and ADR-043.

## 4. Block-device trait

The core needs a deliberately small storage contract, conceptually:

```rust
trait BlockDevice {
    fn geometry(&self) -> Geometry;
    fn read_blocks(&mut self, lba: u64, dst: &mut [u8]) -> Result<()>;
    fn write_blocks(&mut self, lba: u64, src: &[u8]) -> Result<()>;
    fn flush(&mut self) -> Result<()>;
    fn discard(&mut self, lba: u64, blocks: u64) -> Result<()>;
}
```

Additional capabilities are queried rather than assumed:

- reliable flush/barrier
- discard/TRIM
- read-only
- atomic-write unit, if known
- physical alignment
- preferred transfer size

The transaction engine must never silently assume stronger durability than the backend reports.

## 5. Composable test backends

Block backends should be wrappers, not separate filesystem implementations:

```text
FileBackend
   |
TraceBackend
   |
FaultBackend
   |
LatencyBackend
   |
AFS+ core
```

or:

```text
MemoryBackend
   |
TinyCache stress
   |
PowerCutBackend
   |
AFS+ core
```

This permits deterministic tests without contaminating filesystem algorithms with test-only conditionals.

## 6. Rust on Macaros Native

Macaros Native does not need a self-hosted Rust compiler before it can run Rust code.

The first stage is cross compilation on the development Mac:

```text
M5 rustc
   |
AROS AArch64 target
   |
Rust static library / Rust driver binary
   |
M1 Macaros Native
```

For kernel/driver-style Rust, the useful model is `no_std` plus explicit AROS abstractions. Linux follows the same broad principle: Rust code in the kernel links `core`, not normal user-space `std`.

AROS will eventually benefit from first-class Rust driver support, but that is a platform project distinct from AFS+.

Required platform pieces include:

- build-system integration
- target specification and ABI validation
- panic strategy
- allocator interface where `alloc` is used
- safe wrappers for Exec primitives
- device/resource/library wrappers
- interrupt/DMA ownership abstractions for hardware drivers
- module/handler entry-point support
- C/Rust FFI rules

AFS+ can become an excellent first serious consumer of those abstractions because its hardware-facing boundary is only a block-device interface.

## 7. What should remain C-accessible

Even if most reference code is Rust:

- disk-format definitions remain public
- a C API wraps the reference core
- a tiny C reader remains available
- third parties can implement the format independently
- AROS DOS glue can initially remain C

This avoids turning AFS+ into a Rust-only ecosystem.

## 8. Testing advantage

Rust is particularly useful because the host build can combine:

- property tests for encode/decode round trips
- fuzzing of every on-disk record
- model-based allocator testing
- transaction state-machine testing
- deterministic crash injection
- Miri/sanitizer-style host checks where applicable
- randomized operation traces followed by invariant verification

A production AROS bug should ideally become a small host-side reproducer against an image file before the fix is accepted.

## 9. Development principle

The physical AROS handler is an adapter around a filesystem core, not the place where the filesystem is invented.

That separation is one of the main reasons AFS+ can advance before Macaros Native's storage stack is complete.
