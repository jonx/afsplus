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

## 3. UTF-8

AFS+ names are valid UTF-8.

Invalid UTF-8 cannot be created in AFS+.

This is an AFS+ rule, not a requirement placed on every filesystem supported by AROS.

## 4. Normalization-preserving lookup

The proposed canonical comparison normalization is NFC.

The key distinction is:

```text
stored/display name = original valid UTF-8 bytes
comparison key      = canonical key derived from those bytes
```

For a case-sensitive directory, canonically equivalent Unicode spellings compare as the same name according to the frozen normalization algorithm, but the original spelling is preserved.

For a case-insensitive directory, the comparison key additionally applies the specified case-folding algorithm.

AFS+ must not force the stored display name itself into NFC or NFD.

## 5. Unicode version is a format parameter

Normalization and case-fold results can change across Unicode versions.

Every formatted volume therefore records the exact Unicode normalization/casefold table version used for directory-key generation. Writers must use the volume-declared version, not whatever Unicode tables happen to ship with the host OS.

Changing the Unicode table version is a filesystem conversion operation, not an invisible implementation upgrade.

The exact superblock/format-descriptor field is TBD before epoch 1, but the semantic requirement is frozen now.

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
