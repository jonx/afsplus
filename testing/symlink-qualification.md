# Symlink Qualification

> **ADRs:** [ADR-068](../adr/ADR-068-portable-symlink-targets.md) ·
> **Milestones:** M03, M05, M12, M14

## Purpose

Prove that symbolic links are checksummed, bounded namespace objects whose
target bytes survive every implementation unchanged. Path resolution itself
remains an OS/path-layer test; this gate tests storage, object lifetime and
adapter transport.

## Codec and corruption matrix

Run `cargo test -p afsplus-format inline_symlink` for the explicit borrowed-target
codec. Require maximum-length exact round-trip, refusal of every undersized
encoding buffer and every truncated input, plus valid-CRC malformed target,
allocation, flag, reserved-field and unused-tail cases. The ordinary fixed-record
encoder/decoder must reject symlinks until its callers preserve their payloads.
Run `cargo test -p afsplus-format --test symlink_c` on Unix hosts with a C compiler
and address/undefined-behavior sanitizer support. The test compiles the independent
[reader](../portable/c/reader.c) and [probe](../portable/c/tests/symlink_probe.c)
under strict C99 warnings and sanitizers, cross-reads six Rust-generated targets,
and compares refusal of fourteen valid-CRC malformed variants per target.
It checks all truncated input lengths, borrowed target bytes, fixed metadata and
unchanged output parameters on failure. These are private temporary regular-file
fixtures. This host compiler gate does not claim m68k runtime qualification.
Integrated filesystem qualification below is separate.

Rust and portable C cross-read targets that are dangling, relative, absolute,
AROS-qualified, Unicode, contain `.`/`..`, and reach the exact format maximum.
Both reject empty, NUL-containing, invalid-UTF-8 and oversized targets, plus
valid-CRC records with inconsistent payload length, size, flags, allocation
fields, object identity or directory type hint.

Decoders first validate the common header and checked size arithmetic. A short
API buffer returns the exact required count and never exposes a prefix as a
successful result.

## Namespace and crash matrix

For create, final unlink and atomic replacement, inject a cut at every block
write and flush. Recovery must select either the complete old namespace or
the complete new namespace. The selected state passes the exhaustive checker;
no state may expose a directory entry without its symlink object or a target
whose bytes belong to the other transaction.

Rename preserves object ID and exact target bytes. Final unlink retires only
the symlink object and COW metadata, never an extent or orphan entry. Replacing
a symlink creates a new object; hard-link creation for symlink objects returns
the documented unsupported result in the first implementation.

## Payload-preserving rewrite coverage

Exercise every object-record rewrite with a nontrivial inline target, including
ordinary protection changes, exact restore metadata, same-directory rename,
cross-directory rename and replacement. Compare exact target bytes, object ID
and all required metadata after remount and through retained snapshots. A fixed
96-byte re-encoding of a symlink must refuse rather than omit its target.

Require directory leaf validation, object lookup and exhaustive ownership checking
to agree on type hint 3 before enabling writable namespace operations. Targets
must never be interpreted as data extents, entered into regular-file orphan
cleanup, or followed by the object-ID API. Include a valid-CRC malformed target
in metadata-mutation tests and require refusal before publication.

## Adapter and constrained gates

The Rust VFS, FUSE, Hosted MacAROS and independent C API read the same fixture.
FUSE and AROS adapters pass target bytes to their namespace layer without
format-level rewriting. Diagnostic text and JSON escape control characters
deterministically.

The portable C reader uses caller-owned buffers and passes strict warnings,
sanitizers, static analysis, deterministic fuzz replay and the configured m68k
compiler. Qualification records maximum stack/workspace use and the exact I/O
count for a maximum-length target.

## Core namespace and retained-target gates

Run `cargo test -p afsplus-check --test metadata symlink_` for core creation,
metadata edits, rename, remount and unlink. Exercise both snapshot-disabled and
snapshot-enabled volumes. The publication test enumerates modeled crash states
for creation, cross-directory rename, unlink and exact metadata restoration.
Require complete old/new namespace and object metadata, exact captured target
bytes, short-buffer no-copy behavior and exhaustive ownership checking.

Admission tests require zero writes and flushes for malformed/oversized targets,
occupied names, unsupported symlink hard links, read-only mutations and short
live/captured target reads. These tests do not qualify atomic replacement,
provider grants, archive symlink groups or mounted OS adapters. Add those cases
before their respective capability and preservation claims.
