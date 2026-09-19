# 18. Third-Party Integration

> **ADRs:** [ADR-122](../adr/ADR-122-aros-partition-identity.md) · **Spec:** [disk-layout](../spec/disk-layout.md) ·
> **Tests:** [`check-probe-kit.sh`](../tools/check-probe-kit.sh) · **Milestones:** [\[M01\]](../implementation/milestones.md)

What a partition editor, an installer, a `blkid`-style prober or another
filesystem implementation needs from AFS+, and where each thing is.

## 1. Partition identification

An AFS+ partition on a GPT disk has the type
`4146532B-BB67-46C5-AA4A-F502CA018E5E`: the AROS scheme applied to the
DosType `AFS+` ([ADR-122](../adr/ADR-122-aros-partition-identity.md)).
`afsplus-disk wrap` writes such a disk from an image and `afsplus-disk
extract` takes the image out by that type. MBR typing is not used.

## 2. Probing a volume

One read of the first 4096 bytes of the partition identifies an AFS+
volume: the identification block carries a CRC32C over the whole block, the
magic `AFSPLUS1`, the format epoch, the volume UUID, the label, the block
size, the block count and the three feature masks
([disk-layout](../spec/disk-layout.md)). A prober that checks the CRC has
seen a block AFS+ wrote, not a coincidence of bytes.

[`portable/probe/afsplus_probe.c`](../portable/probe/afsplus_probe.c) with
[its header](../portable/probe/afsplus_probe.h) is that prober: C99, no
dependency, BSD-2-Clause, two files to copy into any tree.

```c
#include "afsplus_probe.h"

uint8_t block[AFSPLUS_PROBE_BYTES];      /* read at byte 0 of the partition */
struct afsplus_probe_result r;
char uuid[37];

if (afsplus_probe(block, sizeof block, &r) == AFSPLUS_PROBE_OK) {
    afsplus_probe_uuid_text(r.uuid, uuid);
    printf("AFS+ \"%s\" %s, %llu blocks of %u bytes\n",
           r.label, uuid, (unsigned long long)r.total_blocks, r.block_size);
}
```

The answer is one of `OK`, `NOT_AFSPLUS` (neither the block type nor the
magic is there), `DAMAGED` (AFS+ marks, but the block does not check),
`UNSUPPORTED` (an epoch or layout this probe does not read) or `SHORT`. An
unknown feature bit is reported, not refused: a prober identifies, a mounter
decides. The same file built with `-DAFSPLUS_PROBE_MAIN` is the command
`afsplus-probe [--json] <device-or-image>`, exit 0 for an AFS+ volume;
`--json` gives one object with `schema` `afsplus-probe`, version 1.

## 3. The probe kit

[`tools/check-probe-kit.sh`](../tools/check-probe-kit.sh) (`make probe-kit`)
compiles the probe under `-std=c99 -Wall -Wextra -Werror -pedantic`, builds
the test vectors below with the project's own tools, and checks the probe
against `afsplus-info`, the full reader, on every mountable one. It leaves
the kit in `build/probe-kit/`: the probe, its two source files, `vectors/`
and `expected/` (the probe's answer for each vector, and the reader's
report beside it), for an implementer to take as a whole.

| Vector | Made by | Expected |
|---|---|---|
| empty, one file, Unicode names and label, a sparse file, a directory of 3,000 files, hard links | `mkafsplus`, `afsplus-populate` | recognised; UUID, label, block size, count and features equal to `afsplus-info` |
| journal replay cases | `afsplus-crash-fixtures` | recognised: the identification block is never rewritten |
| corrupt metadata cases | `afsplus-corruption-corpus` | recognised, except the damaged identification block, reported as damaged, and the unknown-feature case, reported with its bit |
| an unknown incompat bit, resealed | the gate | recognised, bit reported |
| a foreign block, a flipped byte in the block, a 100-byte read | the gate | not AFS+, damaged, short |

The gate's negative control: a probe that skips the checksum fails the
flipped-byte vector.

## 4. Partition editors and resizing

Creating, deleting and preserving an AFS+ partition needs nothing beyond
the type GUID. Filesystem-aware operations, the minimum shrink size, resize
and a consistency check, are the tools' job, not the editor's:
`afsplus-check` verifies ([tools-spec](../tools/tools-spec.md#afsplus-check));
`afsplus-resize` and `afsplus-min-size` are [M11](../implementation/milestones.md),
not started.

## 5. Other implementations

An independent reader starts from the executable corpus and the portable C
reader ([docs/17 section 6](17-portability.md#6-independent-portable-c-implementation),
[conformance](../testing/conformance.md)); the probe kit's vectors are the
smallest set to read first.
