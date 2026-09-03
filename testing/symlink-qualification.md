# Symlink Qualification

> **ADRs:** [ADR-068 proposed](../adr/ADR-068-portable-symlink-targets.md) ·
> **Milestones:** M03, M05, M12, M14

## Purpose

Prove that symbolic links are checksummed, bounded namespace objects whose
target bytes survive every implementation unchanged. Path resolution itself
remains an OS/path-layer test; this gate tests storage, object lifetime and
adapter transport.

## Codec and corruption matrix

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

## Adapter and constrained gates

The Rust VFS, FUSE, Hosted MacAROS and independent C API read the same fixture.
FUSE and AROS adapters pass target bytes to their namespace layer without
format-level rewriting. Diagnostic text and JSON escape control characters
deterministically.

The portable C reader uses caller-owned buffers and passes strict warnings,
sanitizers, static analysis, deterministic fuzz replay and the configured m68k
compiler. Qualification records maximum stack/workspace use and the exact I/O
count for a maximum-length target.
