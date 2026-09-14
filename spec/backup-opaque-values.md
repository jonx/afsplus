# Opaque backup value pairs version 1

[ADR-084](../adr/ADR-084-opaque-backup-value-pairs.md) defines the transport.
Each ordinal `N` is canonical unsigned decimal through `2^64-1`.

## Descriptor and data members

The ordinary file `_AROS_BACKUP/metadata/value-N.pax` contains exactly six unique
UTF-8 PAX records. Unknown or missing fields and versions are refused.

| Keyword | Value |
|---|---|
| `AROS.value.version` | `1` |
| `AROS.value.path` | Exact canonical source object path under `files/`, including directory root `files` |
| `AROS.value.class` | `attribute` or `security` |
| `AROS.value.key` | Exact nonempty metadata key, at most 1024 UTF-8 bytes, no NUL |
| `AROS.value.encoding` | Exact nonempty encoding identifier, at most 128 UTF-8 bytes, no NUL |
| `AROS.value.size` | Canonical unsigned 64-bit byte count |

The following effective ordinary file is
`_AROS_BACKUP/metadata/value-N.bin`, with exactly the declared byte count. A local
PAX header `_AROS_BACKUP/metadata/value-N.size` supplies `size` before the binary
member. Its raw binary header size is zero. All three headers use mode `0600`,
zero numeric ownership/time fields and empty link/user/group strings. Descriptor
and size-header payloads use actual raw ustar sizes; their byte limits are checked
before output. Readers validate descriptor and effective binary identities,
fields and size; local PAX headers obey the general stream admission rules.

Source path spelling is exact. Component rules follow the
[envelope contract](backup-envelope.md); the directory root and optional trailing
directory slash are admitted for binding, but the complete consumer must verify
object kind and namespace membership. Neither key nor encoding is a host path.

## Streaming and completion

Descriptor allocation is bounded by explicit PAX limits. Values use caller
buffers, with no allocation derived from their declared size. Source EOF must
agree with the descriptor. Each source read checks the original backup grant.
Any export failure poisons the envelope writer, preventing a completion receipt.

Import checks ordinal, source path, descriptor fields and binary-member size
before beginning a destination upload under its original restore grant. It
uses staged publication under [ADR-083](../adr/ADR-083-staged-opaque-metadata-restore.md).
Failure poisons stream completion and releases private staging. An unverified-input staged value
can publish only after its original reader verifies the terminal digest and EOF.
A [verified scratch replay](backup-spool.md) supplies prior integrity admission
while checking each replayed chunk, permitting one value to publish at a time.
Both modes retain reader-identity binding; another reader cannot release a value. Earlier successful
publications are not rolled back if a later destination publication fails.
The optional Rust `consumer` feature supplies this transport without requiring
the VFS dependency for standalone framing/metadata codecs.

Multiple pending values consume destination staging and handle budgets. The
complete consumer must schedule large inventories within those budgets using
verified replay or another qualified staging strategy;
exhaustion requires explicit failure, never bypassing integrity admission.

This pair protocol does not establish archive-wide object identity, ordinal/key
uniqueness, inventory completeness or whole-job durability. The complete consumer
must enforce these, report partial restoration after any later failure and check
final integrity/EOF before reporting success. Content recovery has a separate
explicit loss-reporting policy under [ADR-078](../adr/ADR-078-backup-preservation-modes.md).


## Recovery validation

[ADR-091](../adr/ADR-091-bound-regular-file-archive-groups.md) requires raw and
effective descriptor/data paths to match their expected ordinal names. A
[recovery inventory consumer](backup-inventory.md#explicit-recovery-consumption)
can validate descriptors and drain exact payloads without staging destination
uploads. Discarding values does not waive binding, ordering, size or integrity
checks and must produce explicit loss accounting.
