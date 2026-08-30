# ADR-051: Make AROS AArch64 platform profiles explicit

Status: Accepted; apple-aarch64 pre-hardware runtime profile qualified

## Context

The same external AFS+ handler is intended for Hosted MacAROS and native
MacAROS on Apple Silicon. Both expose the AROS AArch64 ABI, but they do not
necessarily share all code-generation constraints. Hosted MacAROS reserves
`x18` in Rust and C because Darwin may alter it during host signal delivery.
That constraint must not silently become part of the AFS+ format, VFS API or
portable handler design.

AFS+ also must not require an in-tree AROS driver or an upstream source change.
The platform-specific Rust target, SDK and `std` glues are build inputs to the
external module, not files owned by the filesystem repository.

## Decision

The AROS AArch64 qualification and package scripts consume one explicit,
coherent platform profile. Target SDK contents and host-executed build tools
are separate roots: a cross SDK must never be mistaken for the machine on
which `genmodule` and `collect-aros` execute. The profile specifies:

- the target AROS SDK, host build-tools and compiler-runtime library roots;
- the Rust target JSON and toolchain;
- the Clang link and generated-object targets;
- the architecture/code-generation flags; and
- the directory containing the seven AROS Rust `std` platform glues.

The qualified default is Hosted MacAROS and retains `+reserve-x18` in its Rust
target plus `-ffixed-x18` for every C object. A future native profile must be
provided by the native SDK and must not inherit those settings unless its own
ABI reserves the register.

Every Alpha-0 package includes `build-profile.txt`, `abi-report.txt` and both in
its checksum manifest. The profile records a human-selected profile ID, the SDK
platform, selected targets and flags, and hashes of `target.cfg`, the two host
tools, the ABI auditor, Rust target JSON and each platform glue. The ABI report
requires an AROS OSABI/ABI-version-1 AArch64 `ET_REL` module and rejects every
`x18` or architectural `TPIDR` instruction. The handler, packet translator,
trackdisk adapter and AFS+ static library remain the same sources for all
profiles.

On 2026-08-30 the `apple-aarch64` SDK produced a complete off-tree handler with
profile ID `macaros-native-apple-aarch64-prehardware`. Its target configuration
independently reserves `x18`; the emitted handler nevertheless contains zero
`x18` and zero `TPIDR` instructions. All package checksums and the strict clean
image check passed. The current Rust target JSON and seven `std` glues are the
already-qualified MacAROS AROS-AArch64 inputs, hashed explicitly rather than
silently copied. This proves native-SDK linkage, not by itself native execution
and not an independently maintained native Rust `std` profile. ADR-053
separately records the first execution of that artifact under the native
Apple-AArch64 QEMU runtime.

## Consequences

An upstream AROS merge is still optional: a platform or distribution can build
and install the external `L:` handler with its own profile. Hosted and native
AArch64 artifacts are distinguishable even when their executable headers name
the same AROS architecture.

The profile mechanism alone is not runtime evidence. ADR-053 adds a writable
retained-RAM transport and proves the operation and unload matrix under native
MacAROS QEMU; ADR-054 adds extraction, strict checking and modeled crash replay.
Durable native media, controlled in-guest power cuts and Apple-hardware
execution remain separate gates.
