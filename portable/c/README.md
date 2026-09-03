# Embedding the portable C reader

The reader is a small C99 library for boot, recovery, classic and third-party
integrations. It shares no executable code with the Rust implementation.

<!-- toc -->

- [Integration choices](#integration-choices)
- [ABI and capability policy](#abi-and-capability-policy)
- [Runtime contract](#runtime-contract)
- [Durable intent-log view](#durable-intent-log-view)
- [Validation](#validation)
- [Fuzzing and exact reproduction](#fuzzing-and-exact-reproduction)

<!-- /toc -->

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
- one writable scratch buffer of at least one logical block,
  `AFSPR_TREE_SCRATCH_SIZE` bytes for tree iteration, or
  `AFSPR_INTENT_SCRATCH_SIZE` bytes for intent-log operations; and
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

The object decoder recognizes the per-file private-in-place policy flag from
[ADR-065](../../adr/ADR-065-persistent-data-update-policy.md) without changing
read semantics. A flagged directory, an unknown object flag, or a flagged file
on a volume without `AFSPR_COMPAT_DATA_POLICY` is corruption rather than an
ignored policy.

Every operation is read-only, allocation-free and bounded by the tree-height
or configured log-slot cap. The data destination and scratch buffer must not
overlap. `AFSPR_CAP_FILE_READ` names the selected checkpoint view only;
callers explicitly select the durable overlay through the separate intent
capabilities.

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

## Durable intent-log view

`afspr_scan_intent_log` validates the sequence bound to the selected
checkpoint, verifies every referenced replacement block and returns the exact
valid prefix plus the first excluded tail slot and LBA. A stale, empty,
out-of-sequence or content-damaged tail is reported in `tail_state`; it does
not erase earlier complete records. Device I/O and feature-contract failures
remain hard errors with a structured stage and LBA.

`afspr_lookup_intent_directory_entry` resolves a name against the selected
checkpoint plus the durable prefix. `afspr_intent_directory_next` merges the
committed tree and logged create/rename targets in comparison-key order through
a caller-owned cursor. Deletes, rename chains, case-only renames, hard-link
identity and replacement are reflected. A too-small name buffer leaves the
selected entry pending, so retrying the same cursor cannot skip it.

`afspr_intent_file_size` and `afspr_read_intent_file` expose the corresponding
durable file bytes without writing the device or allocating memory. They
handle files created only in the log, sparse growth, partial-block replacement,
shrink, last-link deletion and identity retained through rename or another hard
link. Each operation validates namespace replay preconditions, including
source and target existence, replacement intent, monotone create IDs and
write/truncate expected sizes.

The caller retains the `afspr_intent_view` and passes it back on each lookup,
cursor step or file read. Every call rescans the bounded log and revalidates
the semantic prefix before using the view; no mutable validation cache or
borrowed media pointer crosses calls. This favors a simple anti-TOCTOU contract
over throughput: work is bounded by the configured log slots and operations,
but an integration performing large directory scans should cache results above
the reader only while it can guarantee an immutable device view.

Legacy identity-key volumes support every valid UTF-8 spelling byte-for-byte.
The versioned Unicode profiles support exact ASCII normalization and ASCII
case folding. A non-ASCII logged name on those profiles clears the advertised
intent views, and a direct non-ASCII lookup returns `AFSPR_ERR_UNSUPPORTED`;
neither is approximated with locale rules. Committed ordinal enumeration keeps
the base reader's structural behavior, whose stored non-ASCII comparison keys
cannot receive a semantic name/key cross-check until frozen Unicode 16 tables
are present.

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

A second Rust fixture leaves seven fsynced records beyond its checkpoint:
hard-link deletion, existing-file write and truncate, create, a two-step rename
chain, another rename and replacement. C scans that prefix, resolves the final
case-insensitive names, enumerates them in key order and reconstructs both
surviving files in 777-byte reads against Rust-written oracle bytes. It also
requires exact record coordinates for missing rename sources, replacement
flag mismatch, non-monotone create IDs and write-size mismatch. Content damage,
a sequence gap and a missing v3 feature bit stop or fail at the exact record or
data LBA. The replacement victim carries Rust's persistent private-in-place
flag on a data-policy volume, while the primary fixture injects the same flag
without its feature and requires object-decode failure.

The same gate tests retained-checkpoint fallback and corrupt/ambiguous states,
compiles the public example, and runs AddressSanitizer/UndefinedBehaviorSanitizer
plus the compiler's static analyzer when supported. If the local AROS m68k
compiler exists, it compiles the library for that target without linking host
facilities. A separate test project consumes the CMake install, so a broken
exported target cannot pass.

## Fuzzing and exact reproduction

Run the bounded mutation gate separately with:

```sh
make portable-c-fuzz-gate
AFSPLUS_FUZZ_RUNS=250000 make portable-c-fuzz-long
```

The seed packer records only blocks actually read during a successful
operation. Its `.afzf` packet carries the virtual device size, operation and
arguments followed by `(LBA, 4096-byte block)` records. This keeps a complete
probe, lookup, directory descent, file read or durable namespace lookup between
12 and 48 KiB instead of copying a 16-MiB image. The packet reader treats
absent blocks as I/O failures and never writes the source packet.

The mandatory smoke engine makes every mutation a pure function of a seed and
case number. It mutates every packet-header byte and the first 128 bytes of
every observed block twice: once as a torn checksum and once with a recomputed
CRC so structural decoders are reached. Later cases mix block-wide flips,
overwrites, multi-byte changes and truncations, resealing alternate cases. If
a sanitizer stops the process, the gate prints a progress file containing the
seed and case. Recreate and preserve the exact input with:

```sh
build/afsplus-fuzz-smoke --case 1234 --artifact failure.afzf seed.afzf
build/afsplus-fuzz-replay failure.afzf
```

The replay line includes each invoked operation's symbolic result plus the
last diagnostic stage and LBA. This makes a small, immutable regression case
that any developer can run. CMake can build `afsplus-fuzz-pack`,
`afsplus-fuzz-replay` and
`afsplus-fuzz-smoke` with `-DAFSPLUS_READER_BUILD_FUZZ_TOOLS=ON`.

[`fuzz_target.c`](fuzz/fuzz_target.c) also exports the standard
`LLVMFuzzerTestOneInput` entry point. When `-fsanitize=fuzzer` is available,
the repository gate invokes it on the same corpus. Some Apple command-line
tool installations omit the libFuzzer runtime; that is reported as a skip,
while the deterministic ASan/UBSan engine remains mandatory.
