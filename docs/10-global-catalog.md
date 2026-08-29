# 10. Global Catalog

## 1. Purpose

The global catalog is an optional non-authoritative, rebuildable index optimized for reading metadata about very large numbers of namespace entries with mostly sequential I/O.

It is inspired by the practical benefit of centralized metadata systems such as the NTFS MFT, but it is not authoritative.

Primary target workloads include:

- Ferail full-volume scans
- desktop search
- backup tools
- duplicate finders
- antivirus/scanners
- package indexing
- fast diagnostic inventory

## 2. Core invariant

The filesystem remains correct and fully navigable without the catalog.

Directory indexes and object records are authoritative.

## 3. Catalog records represent namespace links

A catalog record contains enough metadata for bulk discovery without resolving each path individually:

- object ID
- parent object ID
- object type
- UTF-8 name
- logical size
- selected timestamps
- selected flags
- object generation

Absolute paths are never stored.

A record represents a **namespace link/name**, not a unique object row.

Therefore a file with multiple hard links appears once for each linked name/parent combination:

```text
(parent A, name x) -> object 42
(parent B, name y) -> object 42
```

Consumers that want unique filesystem objects must deduplicate by stable object ID. Consumers that want namespace inventory keep every catalog record.

This distinction is part of the API contract and must not be inferred by applications.

## 4. Rename behavior

Because records use parent IDs, renaming or moving a directory does not require rewriting records for every descendant.

A rename changes the link record for the renamed object, not the stored parent IDs of all descendants.

## 5. Physical layout

The catalog should be stored in large sequential segments.

The reader API returns batches or an iterator. It must not allocate an array containing every file in memory.

## 6. Validity

The filesystem core maintains an authoritative metadata generation.

The catalog records:

```text
catalog_generation
```

If:

```text
catalog_generation != filesystem_generation
```

the catalog is stale unless a defined delta mechanism proves it can be brought current.

Stale catalogs are never silently trusted.

## 7. Writers that do not support the catalog

Because the catalog is non-authoritative and rebuildable, a basic writer may modify the filesystem without updating it.

Such a writer must still update the core metadata generation.

A catalog-aware implementation detects the mismatch and falls back to normal traversal.

## 8. Rebuild

Catalog rebuild must be:

- online where possible
- restartable
- memory-bounded
- safe after interruption

The rebuild writes a new catalog generation and activates it only after complete validation.

## 9. API

Filesystem API v2 exposes semantic bulk enumeration:

```text
FSV2_EnumerateObjects()
```

The final API must make explicit whether enumeration is link-oriented or unique-object-oriented. AFS+ may implement either view efficiently from the catalog, but applications must not guess.

It does not expose catalog blocks.

AFS+ may use the catalog. Another filesystem may use an MFT, optimized traversal, or another mechanism.

## 10. Performance target

The benchmark suite must include multi-million-link sequential catalog scans and compare them with fallback directory traversal.

The specification defines the workload; numeric release targets are set from measured reference hardware rather than guessed in advance.
