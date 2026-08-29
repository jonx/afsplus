# 11. Change Stream

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

## 6. Rescan rule

If a consumer asks for changes older than the oldest available sequence:

```text
FSV2_ERR_RESCAN_REQUIRED
```

The consumer then performs `FSV2_EnumerateObjects()` and records the resulting current sequence.

This fallback is part of the API contract.

## 7. Transaction relationship

Change records become visible only for committed filesystem transactions.

A crash must not expose notifications for operations that never committed.

## 8. Relationship to transient Notify

AROS transient notifications remain useful for live applications.

The change stream complements them by surviving application downtime.

A future notification implementation may use the change stream internally, but applications must not depend on that implementation detail.
