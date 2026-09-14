# Full-preservation object inventory manifests

[ADR-086](../adr/ADR-086-backup-inventory-manifests.md) binds an object's
[opaque values](backup-opaque-values.md) to its inspected
[metadata inventory knowledge](backup-object-metadata.md). This archive
component changes no AFS+ disk record or security evaluation policy.

## Manifest and ordering

The ordinary file `_AROS_BACKUP/metadata/inventory-N.pax` contains exactly these
unique PAX records. Its header uses mode 0600, zero uid/gid/mtime and empty
link/user/group names, with the exact payload length.

| Key | Value |
|---|---|
| `AROS.inventory.version` | `1` |
| `AROS.inventory.path` | Canonical source object path, including directory root `files` |
| `AROS.inventory.attributes` | Unsigned 64-bit attribute count |
| `AROS.inventory.security` | Unsigned 64-bit security value count |
| `AROS.inventory.attribute_bytes` | Unsigned 128-bit sum of attribute sizes |
| `AROS.inventory.security_bytes` | Unsigned 128-bit sum of security sizes |
| `AROS.inventory.attribute_hash` | 64 lowercase hexadecimal digits |
| `AROS.inventory.security_hash` | 64 lowercase hexadecimal digits |

Integers are canonical decimal, with no sign or leading zeros except `0`.
Unknown, missing, repeated or malformed fields refuse the group. PAX record and
payload limits apply before payload allocation. The manifest consumes ordinal
`N`; attribute pairs followed by security pairs consume consecutive ordinals
`N+1` through `N+count`. Each class has strictly increasing exact UTF-8 keys.
The final ordinal may equal the unsigned 64-bit maximum; no next ordinal then
exists. The enclosing job must enforce ordinal uniqueness across groups.

## Descriptor digest

Each class has its own SHA-512/256 digest. Begin with the ASCII bytes
`AROS.inventory.v1` followed by zero and one class byte: zero for attributes,
one for security. For each descriptor in key order append:

1. Little-endian unsigned 64-bit key byte length, then exact UTF-8 key bytes.
2. Little-endian unsigned 64-bit encoding byte length, then exact UTF-8 encoding.
3. Little-endian unsigned 64-bit value size.

Empty classes hash the domain and class bytes alone. Value payload integrity
belongs to the [archive envelope](backup-envelope.md); the descriptor digest
binds identity, order, encoding and declared size. Neither digest authenticates
a sender or proves that an untrusted source enumerated its entire filesystem.

## Captured export

Full preservation requires inspected `Empty` or `Present` knowledge for both
classes. Empty means zero entries; Present requires at least one. Uninspected
state or unsupported required enumeration refuses full preservation.

A descriptor prepass calculates counts, byte sums and hashes through pages of
1 to 64 entries, retaining only a page, the previous key and digest state.
Combined entry/byte limits and ordinal arithmetic are checked before manifest
output. A second enumeration exports the exact values through their original
revocable snapshot authority, comparing counts, sizes and hashes with the
manifest. Failure invalidates archive completion. The second enumeration doubles
descriptor traversal; it avoids memory proportional to inventory size.

## Verified restore and constrained operation

Import requires [verified scratch replay](backup-spool.md), the expected source
path, inspected object inventory knowledge and destination authority. Count and
byte limits are admitted before staging values. Exact consecutive ordinals,
class/key order and descriptor totals/hashes must match. Each complete value
publishes through its own checked grant; only one upload slot is retained.
The final value of each class is withheld if its completed descriptor summary
mismatches. Earlier publications can survive a later failure. Any error poisons
further reader use and yields no inventory-success summary.

Small profiles can use a one-entry page, one-byte transfer buffer and a
512-byte verified scratch chunk, with separately bounded PAX buffers and
scratch storage. This trades more calls and proof I/O for smaller working sets.
An exhausted quota or unsupported destination refuses preservation; it must
never silently discard security metadata. Host backing storage, allocation
failure handling, peak memory and native runtime support require qualification.

A successful group is not whole-job success. The job must bind all object
metadata and groups, reject extras/omissions, preserve namespace and links,
handle sparse data and reservations, reach replay EOF and synchronize the
destination. Explicit content recovery with loss reporting is a separate mode;
it does not relabel uninspected or discarded metadata as an empty inventory.
