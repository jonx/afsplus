# ADR-051: Make AROS AArch64 platform profiles explicit

Status: Accepted for the build boundary; native profile not yet qualified

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
coherent platform profile. It specifies:

- the AROS SDK and compiler-runtime library roots;
- the Rust target JSON and toolchain;
- the Clang link and generated-object targets;
- the architecture/code-generation flags; and
- the directory containing the seven AROS Rust `std` platform glues.

The qualified default is Hosted MacAROS and retains `+reserve-x18` in its Rust
target plus `-ffixed-x18` for every C object. A future native profile must be
provided by the native SDK and must not inherit those settings unless its own
ABI reserves the register.

Every Alpha-0 package includes `build-profile.txt` in its checksum manifest.
The file records the selected targets and flags plus hashes of the Rust target
JSON and each platform glue. The handler, packet translator, trackdisk adapter
and AFS+ static library remain the same sources for all profiles.

## Consequences

An upstream AROS merge is still optional: a platform or distribution can build
and install the external `L:` handler with its own profile. Hosted and native
AArch64 artifacts are distinguishable even when their executable headers name
the same AROS architecture.

The profile mechanism is not evidence that native MacAROS already runs AFS+.
That claim requires a native SDK profile, a bootable runtime and the same
on-target operation, durability and checker gates used for Hosted MacAROS.
