# Compatibility Rules

## Protected deployed images

This contract follows [ADR-123](../adr/ADR-123-deployed-image-compatibility.md).

An image used as an AROS system volume is a compatibility obligation even
before a public release. A registered baseline fixes the accepted bytes and
semantics, not just the integer epoch. Epoch 1 alone does not distinguish
the historical prototype layouts.

The protected baseline is
[`deployed-epoch1/manifest.json`](../crates/afsplus-check/tests/fixtures/deployed-epoch1/manifest.json).
Its reference writer is `0307e86979ca532c0c88dd014d164e094ebb5d48`:
epoch 1, identification version 3, common header version 1, 4 KiB blocks,
168-byte checkpoints without snapshots, and Unicode 16.0.0 comparison
keys. It also retains the exact initializer used for the M1 `System`
volume: 80 KiB followed by zeros to 256 MiB. The initializer's formatter
commit is unrecorded; the bytes, geometry and hash are the authority.

Changes must preserve reading, recovery and safe writing of these images
without an implicit conversion. A decoder may reject corruption; it must
not redefine valid baseline data as corruption to retire a layout. Feature
identities and wire versions are never reused. A new incompatible encoding
requires explicit negotiation and a separately invoked migration that
preserves the original image; mounting is not migration permission.

The historical 96/112-byte checkpoint layouts are not this baseline and
remain unsupported. This contract does not assert completion of the global
epoch-1 freeze gates or qualification of every optional feature.

The [compatibility tests](../crates/afsplus-check/tests/deployed_compatibility.rs)
load immutable bytes rather than the formatter under test. They check
literal hashes/identity, case-insensitive names, data and metadata, ordinary
mutation and remount, pending-log recovery, and refusal of unknown epochs
or incompatible features before any write. The retained M1 initializer
protects its initial geometry and encoding, not the user's later files.
Run `cargo test -p afsplus-check --test deployed_compatibility` for changes
to format, mount, replay, mutation or admission rules. A new baseline is
added alongside existing fixtures; a failed test is never repaired by
regenerating a protected fixture.

## Mount decision algorithm

1. Validate identification record and superblock checksum.
2. Validate format epoch.
3. Read active feature set.
4. For each unknown feature:
   - COMPAT: continue
   - RO_COMPAT: force read-only or fail if RW requested
   - INCOMPAT: fail mount
5. Verify feature dependencies.
6. Apply requested compatibility profile restrictions.
7. If dirty:
   - normal RW: replay journal
   - read-only: follow documented policy
   - NO_CHANGES: never write media
8. Validate authoritative root structures before exposing the volume.

## Tool rule

Repair tools are stricter than normal mounts.

A repair tool must not modify structures controlled by a feature it does not understand.
