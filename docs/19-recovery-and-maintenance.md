# 19. Recovery and Maintenance

## 1. Recovery is an architecture feature

Repair must not depend on undocumented implementation behavior.

Every invariant used by `afsplus-check` is part of the specification.

## 2. Mount modes

At minimum:

### normal

Replay journal and permit writes.

### read-only

Permit recovery-related writes only if explicitly allowed by the platform policy. The exact semantics must be documented.

### no-changes

Absolutely no media writes.

### degraded/recovery

Permit best-effort reads from a damaged volume while clearly reporting untrusted or missing metadata.

This mode must never silently make repairs.

## 3. Shared verifier

The same portable validation routines should be used by:

- normal mount checks
- `afsplus-check`
- FUSE
- image tests
- fuzz harnesses

This reduces the classic problem where filesystem and fsck disagree.

## 4. Repair log

Every repair operation should be able to emit a machine-readable log containing:

- detected invariant violation
- affected object/block IDs
- action taken
- information discarded
- transaction/generation before and after

## 5. Scrub

A future online/offline scrub walks metadata checksums and reports latent corruption.

The initial implementation may be offline only, but the API should separate verification from repair so online checking can be added later.

## 6. Catalog/change-stream recovery

These structures are rebuildable.

Repair should prefer dropping/rebuilding them over risking authoritative object or directory metadata.
