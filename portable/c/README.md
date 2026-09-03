# Embedding the portable C reader

The reader is a small C99 library for boot, recovery, classic and third-party
integrations. It shares no executable code with the Rust implementation.

## Integration choices

For source vendoring, compile [`reader.c`](reader.c) and add
[`../../api`](../../api) plus [`../../spec`](../../spec) to the private include
path. Consumers include only
[`libafsplus_reader.h`](../../api/libafsplus_reader.h); the wire-format header
is an implementation dependency.

For CMake:

```sh
cmake -S portable/c -B build/portable-c
cmake --build build/portable-c
cmake --install build/portable-c --prefix /chosen/prefix
```

An enclosing CMake project may use `add_subdirectory(portable/c)` and link
`AFSPlus::reader`. Set `AFSPLUS_READER_BUILD_EXAMPLE=OFF` when embedding only
the library. An installed package supports:

```cmake
find_package(AFSPlusReader 0.1 CONFIG REQUIRED)
target_link_libraries(your_target PRIVATE AFSPlus::reader)
```

## ABI and capability policy

This additive slice remains reader ABI 1 and packages as version 0.1. Existing
probe calls and the original placeholder `struct afspr_entry` keep their
layout; the operational iterator uses the separate
`struct afspr_directory_entry`. Every output struct carries the ABI version,
and calls that write structs also receive the caller's size. New code checks
`afspr_capabilities()` instead of inferring support from a package version.
None of these reader calls changes Filesystem API v2 or an application-facing
filesystem-neutral capability.

## Runtime contract

The caller provides:

- a logical-block callback returning zero only after it filled the complete
  requested range;
- the actual block count and logical block size of that device view;
- one writable scratch buffer of at least one logical block, or
  `AFSPR_TREE_SCRATCH_SIZE` bytes for directory iteration; and
- storage for the result and, optionally, structured diagnostics.

The bootstrap implementation accepts the prototype's 4-KiB logical blocks.
It performs exactly three successful block reads for a valid probe: the
identification block and checkpoint slots A/B. It allocates nothing, writes
nothing, retains no caller pointer, has no mutable global state and may be
used concurrently when calls use distinct callbacks and scratch buffers.

`afspr_capabilities()` lets an integrator test the additive reader surface.
After `afspr_probe_detailed`, `afspr_lookup_object` performs a bounded object
map lookup, `afspr_directory_entry_at` reads one entry by tree ordinal, and
`afspr_read_file` fills a caller-owned bounded buffer. Direct extents, extent
trees, sparse holes and unwritten mappings are understood. Returned directory
names live in the caller's name buffer. Passing zero name capacity is a sizing
query: `AFSPR_ERR_BUFFER_TOO_SMALL` leaves the required length in
`entry.name_len`.

Every operation is read-only, allocation-free and bounded by the tree-height
cap. The data destination and scratch buffer must not overlap. This slice reads
the selected checkpoint-root view; intent-log replay is a separate capability
and must not be inferred from `AFSPR_CAP_FILE_READ`.

`afspr_probe` is the compact call. `afspr_probe_detailed` additionally reports
the failed stage, LBA, checkpoint slot and independent status of both slots.
The result is complete only on `AFSPR_OK`; diagnostics are valid whenever the
caller supplies a full `struct afspr_diagnostic`. Coordinates that do not apply
are reported as `AFSPR_NO_BLOCK` and `AFSPR_NO_CHECKPOINT_SLOT`, so log output
does not confuse a missing coordinate with a real block number.

The callback is the only platform-specific piece. The
[`probe_file.c`](examples/probe_file.c) example shows a host-file adapter and
prints a diagnostic suitable for bug reports. Native AROS integrations replace
that callback with their bounded device viewport rather than adding host I/O to
the reader.

The tree walker validates CRCs, type/owner identity, committed generation,
level transitions, separator bounds, key ordering, subtree counts, child LBAs
and cycles along the selected path. Object records and selected directory or
extent values receive typed validation. ASCII directory names additionally
cross-check their stored comparison key. Full semantic checking of non-ASCII
keys awaits the frozen Unicode 16 tables; exhaustive whole-tree and allocation
ownership checks remain the repair tool's job rather than ordinary bounded
reads.

## Validation

From the repository root:

```sh
make portable-c-gate
```

The gate cross-reads a Rust image with more than 300 objects, forcing both its
object map and root directory beyond one leaf. It enumerates to a real file,
decodes its object and compares bounded C reads with the original bytes. A
synthetic but wire-valid extent root reuses those Rust-produced data blocks to
exercise a sparse hole and typed extent lookup. Valid-checksum tree identity
and extent-value corruptions must report the exact failing LBA.

The same gate tests retained-checkpoint fallback and corrupt/ambiguous states,
compiles the public example, and runs AddressSanitizer/UndefinedBehaviorSanitizer
plus the compiler's static analyzer when supported. If the local AROS m68k
compiler exists, it compiles the library for that target without linking host
facilities. A separate test project consumes the CMake install, so a broken
exported target cannot pass.
