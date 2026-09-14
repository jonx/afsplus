# Backup archive qualification

> **ADRs:** [ADR-076](../adr/ADR-076-pax-backup-interchange.md), [ADR-078](../adr/ADR-078-backup-preservation-modes.md) · **Spec:** none ·
> **Tests:** none · **Milestones:** M13, M14

## PAX record codec

Run `cargo test -p afsplus-backup` for the bounded extended-header codec in
[pax.rs](../crates/afsplus-backup/src/pax.rs). The
[Oracle pax manual](https://docs.oracle.com/cd/E88353_01/html/E37839/pax-1.html)
specifies decimal record lengths in bytes, including the trailing newline,
and UTF-8 encoding. Values can contain embedded newlines and equals signs;
record boundaries follow the declared lengths. The codec preserves empty values
for interpretation by the profile layer.

Require exact equality with the independent Python tarfile fixture containing
UTF-8, a newline/equal-bearing path and a negative fractional timestamp. Exercise
decimal-length width transitions, every nonempty truncated prefix of each
single-record fixture, invalid UTF-8, embedded NUL, malformed/overflowing lengths,
duplicate keys, trailing garbage and deterministic arbitrary bytes.

The codec contract uses canonical positive decimal lengths and unique keys in
a record block. Reject duplicate keys before any profile interpretation. General
PAX global/local override and alternate character-set handling require a separate
archive-layer decision. Keywords containing NUL, newline or equals sign are
invalid under this strict codec. Values require UTF-8 and exclude NUL.

Callers supply positive byte, record and key limits; a zero value-byte limit
allows only empty values. Admission precedes output allocation. Decoding borrows
keys and values from the input and allocates record-index bookkeeping within the
record count; encoding preflights the complete output byte size. Test every
limit independently in both directions, including exact-boundary admission.
Caller buffer limits, process peak RAM and target runtime need separate evidence.

## Tar framing and independent recovery

Run `cargo test -p afsplus-backup` and
[check-backup-tar.sh](../tools/check-backup-tar.sh). The host gate requires
Python tarfile and bsdtar. It regenerates the retained Python ustar fixture,
recovers the Rust writer's exact ordinary-file bytes with both independent
readers, compares Python-visible header metadata, and requires a changed-payload
negative control to fail its byte oracle. Extraction streams to a private
temporary file; it does not apply archive paths to a host directory.

The [framing module](../crates/afsplus-backup/src/tar.rs) validates the unsigned
header checksum, ustar magic/version, octal numeric fields, fixed UTF-8 name
fields, supported member types, reserved bytes and zero payload padding.
It supports files, directories, links and PAX local/global headers. Unsupported
numeric/header encodings and member types are explicit errors. Long names use
the ustar prefix when representable; the profile layer supplies PAX names and
nonrepresentable numbers through its validated metadata rules.

A reader requires `begin_payload` after each header, selecting its raw size or
an independently validated PAX size override for a regular file. Selection
precedes payload I/O and respects the caller's member-byte bound. Stream payloads
through caller buffers, drain the exact admitted length and check padding before
advancing. An unfinished member returns busy. Errors after consuming stream
bytes poison the stream; subsequent operations cannot resume ambiguously.

Require two zero end blocks and actual EOF, admitting only a configured number
of extra complete zero blocks. Bound the member count independently of member
bytes. The writer preflights a header and size before output, refuses excessive
payload input, and reports partial-write/padding/end-marker/flush errors.
Successful finish establishes framing; destination durability and profile
completion need their own validation.

Tests cover the Python fixture, every truncated prefix of the Rust fixture,
header/padding/end-marker damage, valid-checksum malformed fields, long UTF-8
names, unsupported types, numeric boundaries, member/byte/padding limits,
payload sequencing, maximum 64-bit override arithmetic and partial I/O errors.
A giant declared length test checks arithmetic/state admission only. Full large
and sparse streams require separate workload evidence. Header buffers are fixed
at 512 bytes; name allocations are bounded by field widths. Total runtime RAM
and constrained-target execution require separate measurements.

## Completion envelope

[ADR-081](../adr/ADR-081-ordinary-pax-completion-member.md) and the
[envelope specification](../spec/backup-envelope.md) define version 2.
Run `cargo test -p afsplus-backup` and
[check-backup-envelope.sh](../tools/check-backup-envelope.sh). The host gate uses
OpenSSL with SHA-512/256 support for independent digest verification, Python for
raw tar/count checks, and Python/bsdtar for ordinary file recovery. Set
`AFSPLUS_OPENSSL` for an explicit executable; the script recognizes the existing
Homebrew OpenSSL 3 path on macOS. This executable is a test dependency.

Require exact receipt equality, delayed receipt availability, changed body/header
rejection, every truncated fixture prefix, missing/duplicate/altered terminal
records, wrong counts, unsupported controls, trailing members, unfinished writes
and flush failures. Validate body count/byte limits separately from the bounded
control payloads. Raw body paths reject control collisions and parent traversal;
resolved PAX names and symlink-safe destination behavior require profile checks.

Verify identical digests with the compact portable hash backend by running the
crate tests and independent gate with:

```sh
RUSTFLAGS='--cfg sha2_backend="soft" --cfg sha2_backend_soft="compact"' cargo test -p afsplus-backup
RUSTFLAGS='--cfg sha2_backend="soft" --cfg sha2_backend_soft="compact"' tools/check-backup-envelope.sh
```

[RustCrypto sha2](https://docs.rs/crate/sha2/0.11.0) defines these backend
configuration flags. Keep software/default build artifacts separate during
concurrent qualification. The hash state is incremental; input length is counted
in 128 bits and admitted within the standard's domain. Tests at the maximum
counter validate refusal without output, independently of huge-file workloads.
An integrity receipt provides no sender authentication and cannot replace the
preservation profile or host grants. Native/older CPU execution, peak RAM,
throughput and sustained large streams need separate measurements.

## Integration acceptance

Record decoding establishes syntax only. The complete consumer must enforce the
versioned preservation profile, path confinement, required metadata, payload
integrity and a completion record before declaring a complete backup/restore.
A valid sequence of PAX records or a valid tar prefix cannot establish completion.

Qualify snapshot capture and destination-scoped restoration, independent ordinary
file recovery, sparse layouts, hard-link identity, timestamps, attributes,
security metadata, reservations, revocation, truncated output, conflicting
metadata, unknown requirements and resource pressure under ADR-076/078.
Every full-preservation omission requires refusal; explicit content recovery
reports the losses allowed by its profile. Profile identity,
integrity and completion semantics follow the format decision process before
normative archive fixtures are generated.

## Effective member admission

Run `cargo test -p afsplus-backup member::tests` for effective local-PAX field
validation. The oracle supplies a safe raw header and malicious overriding
paths, verifies hard-link containment and symlink data semantics, exercises
unknown-field refusal, duplicate records and explicit byte budgets, and checks
64-bit numeric boundaries. Exact signed timestamp cases cover nanoseconds,
negative fractions and both signed-second limits without floating-point
conversion. Invalid precision is refused, never rounded.

The [member contract](../spec/backup-envelope.md#effective-ordinary-member-fields)
is an admission prerequisite for the preservation consumer. Its tests do not
qualify sparse/security transport, archive-wide identity or actual restoration.

The [Oracle PAX reference](https://docs.oracle.com/cd/E88353_01/html/E37839/pax-1.html)
describes field overrides and decimal subsecond units. The preservation
interface deliberately refuses precision loss and retains identity fields
without looking up host accounts.
