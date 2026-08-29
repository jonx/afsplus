# ADR-026: Bounded atomic namespace batches

Status: Proposed

## Context

Applications frequently publish a logically related set of files using temporary files, fsync, and carefully ordered renames. The sequence is platform-specific and a crash between renames can expose a mixed version.

Examples include:

- package installation metadata
- editor/project state
- configuration + signature/index files
- application manifests

AFS+ already requires atomic filesystem transactions internally. A bounded user-facing publication primitive may expose useful atomicity without turning the filesystem into a general database.

## Proposed decision

Reserve a Filesystem API v2 extension for bounded same-filesystem namespace/metadata batches.

Conceptual API:

```text
BeginAtomicBatch()
Replace(temp_a, a)
Replace(temp_b, b)
Rename(c, d)
CommitAtomicBatch()
```

## Constraints

- bounded number of operations
- bounded metadata footprint
- same filesystem only
- file payloads must be written before publication
- durability still requires the documented sync contract
- no arbitrary long-running user transaction
- no locks held indefinitely by applications

## Fallback

Applications must be able to use the traditional sequence on filesystems that do not expose the capability.

## Why only Proposed

Before acceptance we need real application cases and must prove that exposing the transaction boundary does not complicate crash recovery, locking, or compatibility disproportionately.