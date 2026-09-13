# 11. Change Stream

> **ADRs:** none · **Spec:** none ·
> **Tests:** [security-scanning-benchmarks](../testing/security-scanning-benchmarks.md) · **Milestones:** M10

## 1. Purpose

The optional change stream provides a persistent ordered record of namespace and metadata changes.

It is intended for:

- Ferail incremental indexing
- Zed worktree refresh
- backup
- synchronization
- desktop search
- auditing
- cache invalidation

The change stream is **non-authoritative and discardable, but it is not reconstructible history**. Current filesystem state can tell us what exists now, not the exact ordered sequence of past committed events.

## 2. Sequence

Every change record has a monotonically increasing 64-bit sequence number.

Sequence values are scoped to the filesystem UUID.

## 3. Record types

Initial record types:

- CREATE
- DELETE
- RENAME
- LINK
- UNLINK
- DATA_MODIFIED
- METADATA_MODIFIED
- XATTR_CHANGED

Records contain object ID and only the additional information required by the event.

## 4. Rename

A rename record should contain:

- object ID
- old parent object ID
- new parent object ID
- old name where required
- new name

Consumers should not need to infer a rename from unrelated delete/create events.

## 5. Retention

The stream is bounded.

Old records may be discarded according to configured retention policy.

The stream publishes:

- oldest available sequence
- newest committed sequence

Discarding old history does not corrupt the filesystem. It can, however, make an incremental consumer unable to continue from its saved cursor.

## 6. Rescan rule

If a consumer asks for changes older than the oldest available sequence, or if the stream was intentionally discarded/reinitialized:

```text
FSV2_ERR_RESCAN_REQUIRED
```

The consumer then performs `FSV2_EnumerateObjects()` and records the resulting current sequence.

This fallback is part of the API contract.

Repair tools must describe this operation as **discard/reset**, not as a rebuild of the lost event history.

## 7. Transaction relationship

Change records become visible only for committed filesystem transactions.

A crash must not expose notifications for operations that never committed.

The stream itself may lag or be absent when the feature contract permits that state, but it must never fabricate committed history.

## 8. Relationship to transient Notify

AROS transient notifications remain useful for live applications.

The change stream complements them by surviving application downtime.

A future notification implementation may use the change stream internally, but applications must not depend on that implementation detail.

## 9. Gap-free handoff and reset identity

Initial enumeration and its saved stream cursor must describe one provable
boundary. Either retain a consistent view or bracket enumeration with change
capture and reconcile mutations before declaring the consumer current. A
cursor sampled only after an unconstrained traversal can silently lose changes.

Reset must invalidate all previous cursors even when the filesystem UUID is
unchanged and numeric sequences repeat. Choose a stream incarnation token or
an equally strong non-reuse rule before freezing the cursor representation.
This requirement refines the existing reset/rescan contract; the representation
is an M10 design gate.

Notification delivery uses bounded queues and occurs after the relevant
transaction is committed. Overflow, cancellation, retention expiry and gaps
must have explicit outcomes. A slow subscriber cannot retain unbounded
transaction state or force callbacks into partially initialized objects.
Subscription plus initial enumeration must close the same gap as persistent
catch-up. See [book-review qualification](../testing/book-review-qualification.md)
for the required race, reset and overflow experiments.
