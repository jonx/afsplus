# 05. Directories and Names

## 1. Directory representation

Directories use a page-based B+ tree keyed by a canonical **comparison key**.

A directory entry contains:

- canonical lookup/comparison key
- original UTF-8 name bytes as supplied by the creator
- child object ID
- child type hint
- entry flags

The object record remains authoritative for file metadata.

The original stored name is not rewritten merely to match the comparison normalization form. AFS+ is normalization-insensitive for lookup according to the volume's declared Unicode rules while remaining normalization-preserving for display/round-trip.

## 2. Why B+ trees

The design must handle directories containing millions of entries without linear scans.

The tree must also support bounded-memory traversal. An implementation should need only a small number of pages in memory for lookup or iteration.

### Executable prototype status

Newly formatted prototype volumes store directories in the shared typed AFST
COW tree. Leaves map the binary comparison key to the original name, child ID,
and type hint. Mount validates only the root; lookup descends one path; the
checker validates and claims the full tree. Create, mkdir, unlink, rmdir,
rename/move, and file hard-link operations address parents by stable object ID
and publish every affected directory path, object record, and object-map
change through one metadata barrier and alternate checkpoint. Cross-directory
rename preserves object identity, rejects topology cycles, and has an
every-write/every-flush power-cut matrix accepting only the complete before or
after namespace.

Identification version 3 now selects a volume-default comparison encoder. The
general formatter defaults to case-sensitive Unicode 16 NFC. AROS images use
Unicode 16 canonical decomposition, full default case folding and NFC
recomposition. Lookups never consult the host locale. Version-1/2 prototype
images retain their historical byte-identity encoder. Per-directory overrides
remain pending.

The tests bulk-build 1,000 typed entries across leaf/internal pages and
exercise 300 root-namespace creates through real transactions, followed by
exhaustive checking, remount, enumeration, and lookup. This removes the old
one-block directory limit. An explicit release qualification inserts 100,000
permuted typed entries, producing a height-three tree with 2,011 nodes, then
streams them in binary order while validating every original name, comparison
key, type hint, and child ID. Boundary lookups are checked separately.

The constrained run retains at most eight final staged images and two decoded
or derived nodes. The latest standalone release run on the development host
completed in 7.44 seconds, with
172,409 device reads, 174,411 provisional spill writes, 172,408 spill reloads,
and 36.5 MB process peak RSS. The RSS includes the 100,000-entry caller-owned
test batch, sparse memory backend, overlay index, allocator, and test harness;
the page-residency counters isolate the engine's full-page cache. The high
spill amplification is the explicit low-memory tradeoff, not the modern-host
default.

A separate every-write/every-flush matrix publishes the exact entry that
changes the root directory from height 1 to 2, then removes it to force sibling
merge and root collapse back to height 1. Every modeled crash image checks and
recovers to exactly the complete pre- or post-transaction namespace.

Opaque resumable directory cookies, atomic replacement, symlinks, and a
formal orphan lifecycle remain later API/format work; they are not hidden
inside the completed Scale-1 directory-capacity gate.

## 3. UTF-8

AFS+ names are valid UTF-8.

Invalid UTF-8 cannot be created in AFS+.

This is an AFS+ rule, not a requirement placed on every filesystem supported by AROS.

## 4. Normalization-preserving lookup

The executable canonical comparison normalization is NFC using the
volume-pinned Unicode 16.0.0 tables.

The key distinction is:

```text
stored/display name = original valid UTF-8 bytes
comparison key      = canonical key derived from those bytes
```

For a case-sensitive directory, canonically equivalent Unicode spellings compare as the same name according to the frozen normalization algorithm, but the original spelling is preserved.

For a case-insensitive directory, the comparison key performs canonical
decomposition, full default non-Turkic case folding, then NFC recomposition.
For example, `Straße` and `STRASSE`, and composed/decomposed spellings of
`Café`, share one key while enumeration returns the creator's spelling.

AFS+ must not force the stored display name itself into NFC or NFD.

## 5. Unicode version is a format parameter

Normalization and case-fold results can change across Unicode versions.

Every formatted volume therefore records the exact Unicode normalization/casefold table version used for directory-key generation. Writers must use the volume-declared version, not whatever Unicode tables happen to ship with the host OS.

Changing the Unicode table version is a filesystem conversion operation, not an invisible implementation upgrade.

Identification version 3 carries the prototype fields. Their epoch-1 wire
placement remains subject to the normal format-freeze review.

## 6. B+ tree ordering is binary, never locale collation

The stored comparison key has a frozen byte encoding.

B+ tree ordering is an unsigned binary byte comparison of that encoded key. It is never host locale collation and never calls a Unicode collation service while walking tree pages.

Consequences:

- two conforming implementations order the same keys identically
- a tiny reader can traverse/enumerate the tree without embedding Unicode collation tables
- Unicode tables are needed to create/validate arbitrary non-ASCII lookup keys, not to understand the physical tree ordering itself

Before freezing epoch 1, key encoding and normalization rules must be tested against:

- Rust
- Git
- macOS behavior
- Linux behavior
- Samba
- classic AROS applications
- Unicode conformance data

## 7. Case policy

AFS+ must support both:

- case-sensitive directories
- case-insensitive directories

The initial proposal allows a volume default plus an optional per-directory override.

Case policy is metadata, not a host-locale decision.

Case-insensitive comparison must use the volume-declared Unicode algorithm/version, not the current locale.

## 8. Maximum name size

Proposed maximum stored component size: 255 UTF-8 bytes.

The comparison-key representation has its own explicit bounded maximum, to be frozen with the key encoding before epoch 1.

Limits must be exposed through Filesystem API v2.

## 9. Reserved syntax

AFS+ stores names only. AROS namespace syntax such as volume colons and Assigns is not stored as filesystem syntax.

The AROS namespace layer is responsible for rejecting names that cannot be expressed safely through the native AROS path syntax.

## 10. Directory cookies

Directory iteration uses opaque 64-bit cookies rather than exposing tree block numbers.

Cookies need only remain valid according to documented iterator semantics. Applications must not persist them as object identity.

Iterator stability under concurrent mutation is intentionally not implied here; it is part of the concurrency/iterator contract that must be defined and tested before epoch 1.
