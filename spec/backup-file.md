# Bound primary regular-file archive groups

[ADR-091](../adr/ADR-091-bound-regular-file-archive-groups.md) binds
[exact object metadata](backup-object-metadata.md),
[allocation and sparse contents](backup-allocation.md) and
[opaque inventories](backup-inventory.md) to one primary regular file. The group
is a namespace-building component, not a whole backup job or a link graph.

## Group profile and order

Every group starts with an ordinary auxiliary file under
`_AROS_BACKUP/metadata/`. Its mode is 0600; uid, gid and mtime are zero;
link, user and group strings are empty. Both raw and effective member paths
match the required ordinal. The name identifies the group version and profile:

| Profile | Initial member name | Required following components |
|---|---|---|
| Full | `file-v1-full-N.pax` | Allocation at N+1, sparse contents at N+2, inventory manifest at N+3 and its consecutive values |
| Recovery | `file-v1-recovery-N.pax` | Allocation at N+1 and sparse contents at N+2; opaque inventories are omitted |

`N` and its successors are canonical unsigned decimal ordinals. Admit the fixed
ordinal prefix before output; inventory value counts must also fit their ordinal
range before that manifest is written. Return the next unused ordinal, or explicit
exhaustion after the maximum u64. Enclosing namespace orchestration must enforce
uniqueness across groups. Unknown versions, mismatched names and missing required
components refuse the group.

The initial payload is exactly the version-1 object metadata PAX record block.
Its kind is `file`, its path is the primary source path, and its modification
timestamp must equal the effective sparse-member timestamp. The allocation
record binds that same path and logical size to sparse coverage. Validate all
these bindings before reservations or content writes.

The profile stored in the archive is separate from the requested restore mode.
Full restore refuses a recovery group before writes. Recovery can consume either
profile. No failure silently selects another profile or weakens a full request.
Conflicting nested allocation and file restore modes are refused.

## Captured export

Read stat and inventory knowledge through the same retained view and grant.
Full export requires inspected Empty or Present inventories; Uninspected cannot
be converted into an empty manifest. Recovery records the actual captured
knowledge while omitting opaque inventories. It does not enumerate unknown
values merely to fabricate a count.

Emit object metadata, allocation/sparse contents and, for a full group, the
complete inventory manifest and values. Recheck stat and inventory knowledge at
the end. A changed captured descriptor or revoked authority poisons the archive
and prevents group/export completion. Inventory prepasses and second-pass
verification follow the inventory specification.

## Restoration and losses

Require verified sparse replay and a destination-scoped fresh empty single-link
regular file. Full mode restores and verifies allocation, then validates and
publishes opaque inventories. Empty inspected inventories require no upload.
Recovery restores sparse contents without reservations. If the archive group is
full, validate and drain its opaque inventory without calling destination upload
APIs. Validate counts, class/key order, descriptor hashes, path/ordinal bindings
and exact payload lengths even when values are discarded. Malformed input never
becomes successful recovery simply because its metadata is unwanted.

After content and opaque operations, apply exact protection and creation,
modification and change timestamps through the original destination grant.
Read back kind, logical size, protection and all three timestamps. Refuse any
mismatch, unsupported representation or rounding. Recovery does not permit silent
truncation of these core fields. The host owns the authority to restore them;
their presence in an archive grants no privilege.

The file report distinguishes requested mode from archive profile. It carries
reservation loss counts/bytes and one of these opaque outcomes:

- Preserved: the inspected inventory summary was validated and published.
- Omitted: captured inventory knowledge plus the validated transported summary
  when recovery consumed a full group. A recovery group has no transported
  summary; Present and Uninspected are explicit losses/uncertainty, not Empty.

The enclosing job persists losses and uncertainty. A returned component report
alone does not satisfy that persistence requirement. Errors poison further
archive use and return no file-success report, but prior reservations, contents
or opaque publications can survive as a partial destination.

## Resource, compatibility and completion boundaries

Caller budgets bound the initial PAX payload, allocation and sparse maps,
opaque inventory counts/bytes, pages, reservations and transfers. The initial
metadata payload is retained through the group, in addition to component working
sets. Descriptor/payload parsing uses admitted record storage; binary values and
sparse contents use the caller buffer. Recovery drains values without staging
uploads. No memory or loop is proportional to logical holes. Large maps and
host scratch limitations need their own spooled/segmented implementations.

Small profiles may choose one-entry pages, small transfer buffers and 512-byte
verified scratch chunks. Provider alignment and edit limits apply. Exhausted
resources refuse the operation; full preservation cannot silently drop metadata.
A provider without opaque support can perform explicit recovery, with losses
reported, but cannot claim full preservation of Present values. Native runtime,
peak-memory and durability qualification are independent of hosted fixtures.

Directories, symlink targets, hard-link aliases and metadata agreement across
aliases, complete namespace enumeration, archive-wide duplicate/omission checks,
EOF, destination synchronization and durable job loss reporting belong to the
enclosing job. A later namespace edit may change timestamps or link counts, so
that orchestration must finalize and verify affected metadata after such edits.
[Scoped created-entry lookup](../docs/13-filesystem-api-v2.md#scoped-created-entry-lookup)
permits revisiting entries without retaining every active object handle.
No filesystem disk record, feature identity or C ABI changes with this group.

See [regular-file qualification](../testing/backup-archive-qualification.md#bound-regular-file-groups)
for the independent metadata/content oracles and failure cases.
