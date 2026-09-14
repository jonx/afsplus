# Backup object metadata payload version 1

[ADR-082](../adr/ADR-082-backup-object-metadata.md) defines the preservation
metadata decision. The [archive qualification](../testing/backup-archive-qualification.md)
contains its executable admission gates.

## Required records

The payload is a unique-key UTF-8 PAX record block with exactly these fields:

| Keyword | Value |
|---|---|
| `AROS.object.version` | `1` |
| `AROS.object.path` | Exact canonical source path under `files/`, or root directory `files` |
| `AROS.object.kind` | `file`, `directory`, `symlink` or `hardlink` |
| `AROS.object.protection` | Canonical unsigned 64-bit decimal preservation value |
| `AROS.object.created` | Exact signed decimal creation timestamp |
| `AROS.object.modified` | Exact signed decimal modification timestamp |
| `AROS.object.changed` | Exact signed decimal metadata-change timestamp |
| `AROS.object.attributes` | `empty`, `present` or `uninspected` |
| `AROS.object.security` | `empty`, `present` or `uninspected` |

All fields are required. Unknown versions, fields and inventory states are
refused. Record order is immaterial. Decimal timestamps follow the exact
[ordinary-member contract](backup-envelope.md#effective-ordinary-member-fields).
Protection is independent of tar mode bits and does not itself authorize
restoration. Paths preserve UTF-8 spelling and obey the envelope's canonical
component rules; only directories may have a trailing slash. Metadata cannot
name the auxiliary archive namespace.

## Binding and completeness

An ordinary auxiliary member carries this payload under
`_AROS_BACKUP/metadata/`. A profile consumer must bind it to the matching resolved
source member, reject missing or duplicate metadata and verify kind agreement.
Modification time must agree with the effective ordinary header. Multiple names
for one hard-linked object require consistent object metadata; archive-local
link relationships determine identity, never destination object numbers.

`empty` asserts a successfully inspected inventory with no entries. `present`
asserts entries that require lossless transport and validation. `uninspected`
explicitly denies knowledge of completeness. Neither parsing a `present` record
nor an envelope integrity receipt proves that required metadata was transported.
Full preservation requires complete inventory evidence and equivalent destination
semantics. Content recovery must report discarded or uninspected inventories.
Missing provider APIs cannot be replaced by an `empty` default.

## Resource and integration contract

Callers supply byte, record-count, keyword and value limits to the PAX codec.
Decoding borrows the admitted payload strings. Encoding validates the complete
metadata object and record budgets before allocating serialized output. No
allocation depends on a source file's declared size. Unknown transports require
an explicit handler or refusal. The object payload alone does not define sparse
transport, auxiliary member ordering, archive-wide bookkeeping or publication;
these require their own integrated preservation gates before reporting success.
