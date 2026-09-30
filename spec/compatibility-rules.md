# Compatibility Rules

## Pre-release format changes

Before the first official release, AFS+ has no obligation to preserve an
internal image, encoding or API. When an incompatible change is the better
design, make it deliberately and update every reader, writer, adapter,
checker and tool in the same change. Do not add legacy readers or automatic
migration solely to keep our own prototype images working. This follows
[ADR-124](../adr/ADR-124-pre-release-format-evolution.md) and the owner's
standing rule in [AGENTS.md](../AGENTS.md).

A format break is not silent: record its reason and compatibility
classification in an ADR, choose the epoch, record version or feature
identity appropriate to the change, update the specification, and refresh
conformance fixtures and their semantic expectations under review. Retired
identities are not reused. A new handler and the image provisioned for it
must come from matching producers; an internal SYS image may be regenerated
as part of that coordinated change. The first official release ends this
pre-release rule and requires an explicit published compatibility contract.

The [reference manifest](../crates/afsplus-check/tests/fixtures/deployed-epoch1/manifest.json)
records writer `0307e86979ca532c0c88dd014d164e094ebb5d48`: epoch 1,
identification version 3, common header version 1, 4 KiB blocks, 168-byte
checkpoints without snapshots, and Unicode 16.0.0 comparison keys. It also
records the exact M1 `System` initializer, 80 KiB followed by zeros to
256 MiB. The initializer's formatter commit is unrecorded; its bytes,
geometry and hash identify this test input, not an indefinite support promise.

The [reference-image tests](../crates/afsplus-check/tests/deployed_compatibility.rs)
load retained bytes rather than the formatter under test. They detect
unintended changes to names, data, metadata, mutation/remount, log replay
and refusal before writes. Run
`cargo test -p afsplus-check --test deployed_compatibility` when those paths
change. A failure requires investigation; it must not be hidden by casually
regenerating expected bytes. An agreed format break updates the fixtures,
hashes, manifest and semantic oracle together, and does not require retaining
old decoders or obsolete images. Existing images need no byte change merely
to adopt this policy.

## Protected deployed images

There is no protected-image compatibility class before the first official
release. This heading is the historical link target used by superseded
[ADR-123](../adr/ADR-123-deployed-image-compatibility.md); the operative rule is
[Pre-release format changes](#pre-release-format-changes).

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
