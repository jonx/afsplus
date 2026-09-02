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
find_package(AFSPlusReader CONFIG REQUIRED)
target_link_libraries(your_target PRIVATE AFSPlus::reader)
```

## Runtime contract

The caller provides:

- a logical-block callback returning zero only after it filled the complete
  requested range;
- the actual block count and logical block size of that device view;
- one writable scratch buffer of at least one logical block; and
- storage for the result and, optionally, structured diagnostics.

The bootstrap implementation accepts the prototype's 4-KiB logical blocks.
It performs exactly three successful block reads for a valid probe: the
identification block and checkpoint slots A/B. It allocates nothing, writes
nothing, retains no caller pointer, has no mutable global state and may be
used concurrently when calls use distinct callbacks and scratch buffers.

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

## Validation

From the repository root:

```sh
make portable-c-gate
```

The gate cross-reads a multi-generation Rust image, tests retained-checkpoint
fallback and corrupt/ambiguous states, compiles the public example, and runs an
AddressSanitizer/UndefinedBehaviorSanitizer build plus the compiler's static
analyzer when supported. If the local AROS m68k compiler exists, the same gate
also compiles the library for that target without linking host facilities. The
CMake install is consumed by a separate test project, so an install that
exports a broken target cannot pass.
