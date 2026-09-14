# ADR-088: Keep sparse stored size in the raw tar header

Status: Accepted
Amends: ADR-087, ADR-081

## Context

The independent Python sparse extraction oracle rejected a GNU sparse 1.0
entry whose local PAX `size` field carried the condensed map-plus-data size.
Python applies that field after sparse handling, replacing logical size and
recomputing the following-header offset from the wrong data origin. The
interop fixture must recover exact contents and continue through later entries.

## Decision

Sparse entries carry stored size in the raw header, without a local PAX `size`
override. Their logical size is exclusively `GNU.sparse.realsize`. Sparse-aware
admission refuses a conflicting `size` record instead of guessing precedence.

Encode raw size as conventional octal when representable, otherwise as a
positive GNU binary number in the 12-byte size field: leading `0x80`, zero
high-order padding and unsigned big-endian value. Admit only nonnegative values
representable in unsigned 64 bits, with normal checksum validation. Other raw
numeric fields retain the existing octal contract. Ordinary PAX size overrides
retain their existing semantics outside sparse entries.

This combines GNU sparse metadata with a documented GNU size-field extension,
not a strict POSIX-only archive claim. It avoids a stored-data limit at the
33-bit octal boundary while preserving explicit archive/provider resource limits.

## Qualification

Keep the original conflicting-size case as a refusal regression. Require Python
and libarchive content recovery across mixed, all-hole and empty sparse members,
including the archive completion member after them. Compare octal-boundary and
unsigned-64-bit header encodings with Python's independent numeric codec; reject
negative and overflowing binary fields. Large header tests prove representation,
not a sustained multi-gigabyte archive transfer or native large-file support.
