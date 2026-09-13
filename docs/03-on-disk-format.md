# 03. On-Disk Format

> **ADRs:** [ADR-064](../adr/ADR-064-intent-log-data-update-compatibility.md) · **Spec:** [disk layout](../spec/disk-layout.md) ·
> **Tests:** [conformance](../testing/conformance.md) · **Milestones:** M00, M02

## 1. Encoding

All multi-byte integer fields are little-endian.

On-disk structures are byte encodings, not native C structs. Implementations must use explicit load/store helpers.

Required helpers include:

```c
uint16_t afsp_get_le16(const void *);
uint32_t afsp_get_le32(const void *);
uint64_t afsp_get_le64(const void *);

void afsp_put_le16(void *, uint16_t);
void afsp_put_le32(void *, uint32_t);
void afsp_put_le64(void *, uint64_t);
```

## 2. Logical block size

The default logical block size is 4096 bytes.

The superblock stores a block shift. Version 1 permits a bounded power-of-two range, initially proposed as 4096 through 65536 bytes.

Implementations may support a smaller subset but must reject unsupported block sizes before modifying the volume.

## 3. Superblock placement

AFS+ stores multiple checksummed superblock copies at deterministic locations.

Proposed layout:

- primary superblock in the initial metadata area
- secondary copy near the start of the second allocation region
- tertiary copy near the end of the volume

Exact offsets are format constants and must be frozen before format epoch 1.

No superblock copy is updated in place until a newer valid generation has been durably written elsewhere.

## 4. Superblock contents

Required information:

- magic
- format epoch
- structure size
- filesystem UUID
- volume label reference
- logical block shift
- total logical blocks
- allocation region size
- root object ID
- object-tree root
- allocation-region table root
- journal descriptor
- current committed transaction
- metadata generation
- clean/dirty state
- feature records
- checksum type
- superblock checksum

## 5. Magic and identification

A fixed magic and a fixed identification region must permit external tools to recognize AFS+ without parsing arbitrary filesystem metadata.

The identification region must contain enough data for:

- filesystem name
- format epoch
- UUID
- volume label or label reference
- block size
- feature summary
- clean/dirty state

Prototype identification version 3 stores three 64-bit mount-time feature
summaries (`COMPAT`, `RO_COMPAT`, `INCOMPAT`), the directory comparison-key
algorithm and its three-byte Unicode table version. Unknown values are rejected
before checkpoint replay or any other write. Version-1/2 prototype images
remain readable and writable with their explicit legacy byte-identity key
behavior; version 1 additionally derives its intent-log feature bit.

The experimental incompatible assignments include bit 0 for the base intent
log, bit 1 for version-3 existing-file data updates, and bit 2 for persistent
snapshot ownership. Bit 2 binds the registry/lifetime checkpoint extension in
[ADR-073](../adr/ADR-073-snapshot-checkpoint-roots.md).
Bit 1 requires bit 0. The split prevents an older namespace-only replay
implementation from interpreting an unknown but valid data-update record as a
torn tail; see [ADR-064](../adr/ADR-064-intent-log-data-update-compatibility.md).
The numeric values and record layout remain unfrozen until M14.

## 6. Checksums

All core metadata blocks are checksummed.

The initial checksum proposal is CRC32C because it is widely implemented, fast in software, and commonly accelerated by modern CPUs.

The checksum algorithm field remains explicit so a later format can add alternatives.

A checksum mismatch is an integrity failure. Implementations must not continue parsing a metadata block as if it were valid.

## 7. Structure headers

Every independently addressable metadata object begins with a common header containing:

```text
magic/type
header version
flags
object/owner identifier where applicable
transaction generation
payload length
checksum
```

This permits repair tools to classify blocks without relying entirely on external context.

## 8. Format epoch versus feature flags

The format epoch changes only for transformations that cannot reasonably be negotiated through feature flags.

A filesystem should not advance the epoch for ordinary optional features.
