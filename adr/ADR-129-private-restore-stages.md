# ADR-129: Keep retained restore stages private until publication

Status: Superseded
Amends: ADR-127, ADR-128
Superseded by: ADR-130

## Context

A failed copy and failed cleanup can retain partially restored historical data
in the live namespace. Classic protection bits are metadata; current ordinary
AROS open/read does not evaluate them. Filenames and a reported stage prefix
provide neither historical authority nor a private namespace.

## Decision

Mark the standalone staging file, or the staging tree root, with the opaque
internal attribute `system.restore.stage` before copying historical bytes.
Ordinary VFS access refuses the marked object and every descendant of a marked
directory, including requests naming a descendant by object identifier.
Ordinary enumeration skips private stages while continuing through live entries.
Ordinary metadata, attribute, namespace and clone operations cannot remove the
marker or escape the protected subtree. The marker uses existing attribute
records and changes neither format version nor legal filename rules.

The independently authorized restore implementation uses trusted core access
for copying, verifying and cleaning its own stage. Final non-replacing
publication removes the root marker in the same transaction that moves the
verified object to its final name. A retained stage stays private after remount;
a complete published file or tree has exactly its captured attributes and
metadata. Child publication within a private tree does not expose that tree.

The generic core remains a trusted maintenance interface. This rule protects
ordinary VFS and DOS operations, and claims no isolation against shared-memory
corruption or direct disk/core access. Existing backup and restore admission
remain independent requirements.

## Qualification

Retain a real stage through copy and cleanup I/O failures, remount, and refuse
ordinary open/read, object-ID lookup and metadata or namespace mutation. Keep
root enumeration usable. Test marked-tree descendants and object-ID access,
marker removal attempts, exact final file/tree metadata and attributes, and
publication failure outcomes. Retain the original exposing implementation as
an independent negative control. Resource evidence records traversal cost;
metadata payload limits do not describe total peak heap use.
