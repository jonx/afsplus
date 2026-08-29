# 10. Global Catalog

## 1. Purpose

The global catalog is an optional derived index optimized for reading metadata about very large numbers of objects with mostly sequential I/O.

It is inspired by the practical benefit of centralized object metadata systems such as the NTFS MFT, but it is not authoritative.

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

## 3. Catalog record

A catalog record should contain enough metadata for bulk discovery without resolving each path individually:

- object ID
- parent object ID
- object type
- UTF-8 name
- logical size
- selected timestamps
- selected flags
- object generation

Absolute paths are never stored.

## 4. Rename behavior

Because records use parent IDs, renaming or moving a directory does not require rewriting records for every descendant.

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

Because the catalog is derived, a basic writer may modify the filesystem without updating it.

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

It does not expose catalog blocks.

AFS+ may use the catalog. Another filesystem may use an MFT, optimized traversal, or another mechanism.

## 10. Performance target

The benchmark suite must include multi-million-object sequential catalog scans and compare them with fallback directory traversal.

The specification defines the workload; numeric release targets are set from measured reference hardware rather than guessed in advance.
