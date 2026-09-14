# Bound directory, symlink and hard-link archive groups

[ADR-093](../adr/ADR-093-directory-and-hardlink-archive-groups.md) defines
namespace components, extended by [ADR-095](../adr/ADR-095-bound-symlink-archive-groups.md)
for symlinks, alongside [primary regular-file groups](backup-file.md).
The enclosing job owns complete enumeration, parent placement and link graphs.

## Wire binding

An ordinary auxiliary member under `_AROS_BACKUP/metadata/` contains the exact
[version-1 object metadata](backup-object-metadata.md). Its name is
`directory-v1-{full|recovery}-N.pax`, `symlink-v1-{full|recovery}-N.pax`
or `hardlink-v1-{full|recovery}-N.pax`.
Require canonical decimal ordinals, matching raw/effective auxiliary paths,
mode 0600, zero uid/gid/mtime and empty link/user/group fields.

The source member consumes N+1. Its local PAX header is
`_AROS_BACKUP/metadata/namespace-N+1.pax`, with path and exact mtime; hard links
also carry linkpath. Its synthetic raw name is `files/namespace.N+1`.
Directory mode is 0700 and hard-link mode is 0600. Both have zero payload,
zero uid/gid and empty user/group strings. The raw alias link is `files/primary`;
its effective linkpath identifies the canonical primary source path.
Validate effective kind, path, time and target against the descriptor and the
namespace plan, and bind both header names to the expected ordinal.

Full directories append a complete inventory at N+2 and consecutive values.
Recovery directories omit inventories while retaining captured knowledge.
Aliases never duplicate primary contents, allocation or opaque inventories.
Admit ordinal arithmetic before output and return explicit exhaustion after
u64 maximum. Unknown profiles/versions and inconsistent bindings refuse the group.

## Export and restore

Export stat and inventory knowledge through the same captured view and grant,
then recheck both after emission. Full export requires inspected inventories.
Any error poisons archive completion.

Restore requires verified replay. Full restore refuses recovery groups.
A directory target must be scoped, of directory kind and empty, as established
by the optional [bounded emptiness query](../docs/13-filesystem-api-v2.md#scoped-directory-emptiness).
Unsupported queries refuse restoration. Full mode validates and publishes opaque
inventories; recovery validates and drains full inventories without publication.
Apply exact core metadata and verify kind, protection and all three timestamps.
Return preserved inventory or explicit omitted knowledge and transported summary.

For aliases, the namespace plan supplies an already restored primary handle,
its canonical source path and captured inventory knowledge. Require matching
primary kind, protection, timestamps and knowledge, distinct primary/alias paths,
and an alias basename matching the supplied destination name. Full profiles
require inspected knowledge. Preflight a missing destination name and available
lookup capacity before creating a link without overwrite. Reopen the alias,
restore core metadata and verify identical object ID, size, allocated size,
exact metadata and a link count increased by one. Every operation uses the
original scoped restore grants. An alias report does not certify preservation
of the primary's contents or opaque metadata.

## Symlink target preservation

Symlinks use zero-payload ordinary members with mode 0600 and an explicit local
PAX `linkpath`. The raw header contains the namespace placeholder `files/primary`;
missing local `linkpath` is invalid. Preserve target spelling exactly, including
absolute and parent syntax, without resolving or normalizing it. Full groups
append inventories at N+2; recovery groups retain captured inventory knowledge.

Export reads the captured target through the original grant, admits its required
length against the record-byte budget, and requires nonempty NUL-free UTF-8 with
length equal to captured logical size. Recheck target bytes, stat and inventory
knowledge before completing the group.

Restore validates group/profile/ordinal bindings and the destination basename
before creating a fresh scoped symlink. Apply inventory handling for the requested
mode and exact core metadata. Read back the target and verify exact bytes, size,
symlink kind, one link, protection and all timestamps. Return the created handle
and explicit preservation or omission report. A target mismatch fails restoration.

Full restoration with an opaque value needs slots for the parent, created symlink
and one staged upload. Slot exhaustion fails explicitly and releases temporary
handles; it can leave a partial symlink if creation preceded the failure. Recovery
never silently replaces full preservation to fit a smaller handle budget.

## Resources and completion

Record, ordinal, inventory, transfer-buffer and active-handle limits apply.
Directory groups can use a one-byte transfer buffer when providers permit it;
alias creation needs an available lookup handle in addition to primary/parent
handles. The AFS+ emptiness query reads at most one directory entry and allocates
no additional restore handle. Resource refusal never selects recovery implicitly.
Older systems can use smaller admitted buffers and scoped handle reopening;
whole-system RAM, native runtime and durability require separate qualification.

Errors poison further reader use but can leave partial destination mutations.
The enclosing job validates every parent binding, chooses canonical primaries,
rejects duplicate/omitted entries and inconsistent aliases, finalizes directory
metadata after child edits, persists losses, verifies EOF and synchronizes the
destination. AFS+ opaque storage and whole-job symlink namespace safety have
separate gates.
No filesystem disk record, feature identity or C ABI changes with these groups.

See [namespace qualification](../testing/backup-archive-qualification.md#directory-and-hard-link-groups).
