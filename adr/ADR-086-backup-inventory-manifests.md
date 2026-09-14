# ADR-086: Bind complete object inventories to counted descriptor manifests

Status: Accepted under the owner's delegated recommended-option authority; complete backup-job qualification required
Amends: ADR-082, ADR-084, ADR-085

## Context

An individually valid opaque value does not prove that all captured metadata
was archived or that a restore did not repeat a key. Large inventories also need
resource admission without retaining every descriptor in memory.

## Decision

For full-preservation object inventories, enumerate the captured descriptor
pages once to calculate per-class entry counts, value-byte totals and ordered
SHA-512/256 descriptor hashes. Reject uninspected inventories or unsupported
required enumeration before emitting the inventory. Admit total entries/bytes
and the required ordinal range before archive output.

Emit a versioned manifest, then attribute pairs followed by security pairs in
strict UTF-8 key order. Enumerate the same retained view a second time to export
values and require the resulting counts/totals/hashes to match the manifest.
Any discrepancy or source failure invalidates archive completion. This is linear
extra descriptor I/O with bounded page/key memory, not a complete in-memory list.

Import requires verified scratch replay. Match the source path and expected
object inventory knowledge, admit resource limits, enforce sequential ordinals,
class order and unique increasing keys, and compare complete descriptor hashes
and totals. Release each checked upload after publication. Any error prevents a
successful inventory outcome and invalidates further reader publication. Earlier
published values may remain; report partial restoration, not rollback or success.

The manifest occupies one ordinal even for an empty inventory. Value pairs use
the following consecutive ordinals. Exhausting the final ordinal is explicit.
The wire contract is [inventory manifests](../spec/backup-inventory.md).

## Compatibility and qualification

No AFS+ disk record, ABI or ACL evaluation changes. This is a full-preservation
inventory component. Content recovery requires its own explicit loss-reporting
orchestration; uninspected state must not be relabeled empty. Whole backup jobs
must also validate object metadata, namespace/link identity, sparse semantics,
all inventory groups, final archive completion and destination synchronization.

Require empty and paginated multi-entry inventories, one-upload-slot restore,
exact identities/bytes, source mutation or descriptor disagreement, uninspected
refusal, count/hash/ordering corruption and quota/ordinal boundaries. Preserve
bounded key/page memory and record descriptor enumeration cost. Native storage
and sustained workload gates remain separate.
