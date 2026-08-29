# ADR-033: Sealed immutable content

Status: Proposed

## Context

Many files become immutable after publication: Git packfiles, model weights, package objects, checkpoints, media assets, downloaded artifacts, and backup chunks.

Once content is immutable, several expensive tasks can become stable and reusable:

- content fingerprinting
- antivirus/security verdicts
- reflink content identity
- deduplication research without repeated hashing
- long-lived placement decisions
- aggressive read-only caching

Ordinary filesystem immutability flags are useful but AFS+ can define a clearer cross-platform content-generation contract.

## Proposed decision

AFS+ prototypes a `SealContent` operation.

A sealed regular file:

- rejects ordinary writes/truncates
- preserves its current content generation
- may acquire a stable derived content fingerprint
- may be reflink-cloned without losing content identity
- remains renameable/movable subject to namespace/security policy
- retains normal ACL/security metadata semantics

Unsealing, if supported, must be explicit and authorized. It creates or prepares a new mutable content generation and invalidates any derived identity, scanner verdict, or cache that depended on the sealed generation.

Sealing is not equivalent to cryptographic authenticity. A content hash proves bytes, not who authorized them.

## Format impact

Do not require a complex new object layout.

The baseline needs only a versioned object state/flag plus generation semantics. Fingerprints remain optional derived metadata.

## Consequences

This is useful beyond LLMs and should be evaluated against classic immutable-file semantics before acceptance.
