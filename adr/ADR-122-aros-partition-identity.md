# ADR-122: An AFS+ partition is an AROS GPT partition of DosType AFS+

Status: Accepted
Amends: ADR-045

## Context

[ADR-045](ADR-045-native-aros-handler-shell.md) gave the AROS handler the DOS
type `0x4146532b` ('AFS+') for its first DOSDriver mounts. Booting from AFS+
needs more: a partition that AROS recognises on a disk, whose handler the boot
scan finds before any `SYS:` exists. Macaros Native, bare-metal AROS on Apple
Silicon, installs from macOS onto a GPT disk and records "assign its reviewed
GPT identity" as the step before it can format a `SYS:` partition.

AROS already has a GPT scheme. `rom/partition/partitiongpt.c` reads a type GUID
of the form `{DosType}-BB67-46C5-AA4A-F502CA018E5E` as an AROS partition, takes
its DosType from the first field, its boot priority from the low byte of the
upper attribute word and its bootable flag from bit 60. The boot scan
(`rom/dosboot/bootscan.c`) then gives the partition the handler that
`FileSystem.resource` holds for that DosType, and the handler's generated
resident init registers AFS+ there
([`tools/check-hosted-aros-resident.sh`](../tools/check-hosted-aros-resident.sh)).

## Decision

1. The DosType of AFS+ is `0x4146532B` ('AFS+') in every place AROS keeps one:
   the DOSDriver, `FileSystem.resource`, RDB partitions and GPT partitions.
2. The GPT partition type of AFS+ is `4146532B-BB67-46C5-AA4A-F502CA018E5E`,
   the AROS scheme applied to that DosType. No private GUID is minted, and
   partition.library needs no change.
3. A bootable AFS+ partition sets bit 60 of its attributes and its boot
   priority, a signed byte, in bits 32 to 39.
4. Partitions start on a 1 MiB boundary. The file system inside keeps its own
   block size (4096 by default); the partition carries 512-byte sectors.
5. `afsplus-disk wrap` writes such a disk from a formatted AFS+ image, and
   `afsplus-disk extract` takes the partition out again by its type. The
   Native installer uses the same layout.

## Consequences

- Any AROS with partition.library and the AFS+ handler as a boot module finds
  and boots an AFS+ partition with no further change.
- Tools that do not know AROS show the partition as an unknown type
  (`sgdisk` prints `FFFF`); macOS leaves it alone.
- The DosType is also the key a future AROS-wide registry would record; if
  AROS assigns another value, both the DosType and the GUID follow it.
