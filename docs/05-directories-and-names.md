# 05. Directories and Names

## 1. Directory representation

Directories use a page-based B+ tree keyed by a normalized comparison key.

A directory entry contains:

- normalized lookup key
- original UTF-8 name
- child object ID
- child type hint
- entry flags

The object record remains authoritative for file metadata.

## 2. Why B+ trees

The design must handle directories containing millions of entries without linear scans.

The tree must also support bounded-memory traversal. An implementation should need only a small number of pages in memory for lookup or iteration.

## 3. UTF-8

AFS+ names are valid UTF-8.

Invalid UTF-8 cannot be created in AFS+.

This is an AFS+ rule, not a requirement placed on every filesystem supported by AROS.

## 4. Normalization

The proposed canonical normalization is NFC.

Normalization occurs before lookup and storage of the comparison key.

The original display spelling is retained after normalization according to the final case policy.

Before freezing epoch 1, normalization rules must be tested against:

- Rust
- Git
- macOS behavior
- Linux behavior
- Samba
- classic AROS applications
- Unicode conformance data

## 5. Case policy

AFS+ must support both:

- case-sensitive directories
- case-insensitive directories

The initial proposal allows a volume default plus an optional per-directory override.

Case policy is metadata, not a host-locale decision.

Case-insensitive comparison must use a defined Unicode algorithm and version, not the current locale.

## 6. Maximum name size

Proposed maximum component size: 255 UTF-8 bytes after normalization.

The limit must be exposed through Filesystem API v2.

## 7. Reserved syntax

AFS+ stores names only. AROS namespace syntax such as volume colons and Assigns is not stored as filesystem syntax.

The AROS namespace layer is responsible for rejecting names that cannot be expressed safely through the native AROS path syntax.

## 8. Directory cookies

Directory iteration uses opaque 64-bit cookies rather than exposing tree block numbers.

Cookies need only remain valid according to documented iterator semantics. Applications must not persist them as object identity.
