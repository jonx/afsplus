# ADR-085: Verify a bounded scratch archive before incremental restoration

Status: Accepted under the owner's delegated recommended-option authority; full inventory and native qualification required
Amends: ADR-084

## Context

Waiting for archive EOF while retaining every metadata upload makes restore
handle and staging requirements grow with the inventory. A verified scratch
archive permits bounded replay, but replaying mutable storage without checking
its bytes could install data changed after the initial integrity pass.

## Decision

Copy the archive into host-owned scratch storage with explicit archive, scratch
and chunk-size limits. Build a SHA-256 Merkle tree over fixed-size chunks and
retain its root, length and geometry privately in memory. Verify the captured
archive's envelope, effective member framing and EOF before returning a replay
capability. No unverified scratch copy may produce that capability.

Replay validates each chunk's length, padding, index and inclusion path against
the retained root before returning any bytes. The tree uses separated leaf and
internal-node domains as described in [RFC 6962 section 2.1](https://www.rfc-editor.org/rfc/rfc6962.html#section-2.1);
chunk index and actual length are part of each leaf's input. The SHA-256 inputs
are bounded chunks or pairs of hashes, not a whole 64-bit archive. The archive's
SHA-512/256 envelope contract is unchanged.

A replay reader created only by the verified scratch owner may release a staged
value after that value's checked replay, without retaining other uploads until
a second EOF. A staged value remains bound to its reader identity. Replay errors
prevent further release and completion. Previously published verified values may
remain after a later storage or destination failure; report partial restoration.
Whole-job success still requires complete inventory validation, replay EOF and
qualified destination synchronization.

Use caller-provided read/write/seek scratch storage. Keep only one chunk buffer
and logarithmic tree-layout metadata in the replay cursor; tree hashes live in
scratch storage. This is an ephemeral host mechanism, not an AFS+ disk format or
a persistent restart protocol. Scratch quotas include tree and padding overhead.
No scratch handle or root mutation API is exposed to the consumer.

## Alternatives and qualification

Keeping every upload open was rejected for large constrained inventories.
Rechecking the entire archive before every value would bound memory but produce
quadratic I/O. Trusting a writable scratch file after one hash pass would miss
later corruption. Merkle proofs retain bounded per-read working memory and
logarithmic proof I/O without requiring secret keys or an entropy source.

Require independent root calculation, odd-tree and partial-chunk cases, modified
or reordered data and hashes, truncation, read/write/seek/flush errors and quota
refusal. Test a real temporary regular file as well as a faultable memory store.
Restore more values than the handle budget permits simultaneously by publishing
one verified value at a time. Measure scratch overhead and maximum read size;
keep host evidence separate from native durability and full workload measures.
