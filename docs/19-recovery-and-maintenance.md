# 19. Recovery and Maintenance

> **ADRs:** none · **Spec:** none ·
> **Tests:** [conformance](../testing/conformance.md),
> [developer-harness](../testing/developer-harness.md) · **Milestones:** M05, M11

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

The catalog is rebuildable from authoritative objects and directory links.
Repair may discard it and rebuild a complete generation before activation.

The change stream is discardable, but its lost ordered history cannot be
reconstructed from current state. Discard/reset invalidates old cursors and
requires a full rescan. It must never fabricate replacement historical events.
See [change-stream semantics](11-change-stream.md) and the
[normative invariants](../spec/invariants.md#change-discovery).

Neither operation may risk authoritative object or directory metadata.
