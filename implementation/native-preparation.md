# Preparing AFS+ for Macaros Native

Goal: Macaros Native, bare-metal AROS on Apple Silicon, boots from an AFS+
`SYS:` and shows what the v2 interface gives an application. Native has no
disk driver yet (see its `docs/MILESTONES.md`), so everything here is built
and proven on Hosted Macaros and under QEMU, in the form Native will take
unchanged. The Native agent owns the AROS platform code and the boot bundle;
AFS+ hands over the pieces below through the board.

Starting point: hosted Macaros runs its desktop from AFS+ after a pivot of
`SYS:` during startup (S1b, `tools/check-hosted-aros-s1b.sh`). The boot itself
still comes from the host folder.

## Facts the plan rests on

- AROS registers disk-based handlers in `FileSystem.resource` by DosType. A
  module built by genmodule with a `##begin handler` section does it from its
  resident init (`tools/genmodule/writestart.c`, `InitHandler`): a seglist
  from the handler function, one `FileSysEntry` per DosType.
- The boot scan (`rom/dosboot/bootscan.c`) reads RDB and GPT partitions of
  every disk with a boot node, gives each partition the handler that
  `FileSystem.resource` holds for its DosType, and adds a boot node at the
  partition's boot priority. `bootdevice=NAME` selects one.
- AROS GPT partitions carry their DosType in the type GUID:
  `{DosType}-BB67-46C5-AA4A-F502CA018E5E`, boot priority in the low byte of
  the first flags word (`rom/partition/partitiongpt.c`). The AFS+ DosType is
  `0x4146532B` ('AFS+'), so the AFS+ partition type is
  `4146532B-BB67-46C5-AA4A-F502CA018E5E`. partition.library needs no change.
- Hosted has one device that registers disks at boot, `hostdisk.device`, and
  its automount probes a fixed `/dev/disk%ld`: the Mac's physical disks. It
  must not be loaded as it is.

## Work, in order

1. **Resident handler.** The handler object also carries a ROMTag whose init
   registers it in `FileSystem.resource` for DosType `AFS+`, as genmodule
   does. Loaded from `L:` it behaves as today. Proof: a hosted boot lists it as
   a boot module, and a DOSDriver without a `FileSystem` line mounts an AFS+
   volume through the registered entry; without the module the same mount
   fails.
2. **Partitioned AFS+ images.** A host tool writes a GPT disk image with an
   AFS+ partition of the type above, a boot priority, and a formatted and
   populated AFS+ file system inside it, and checks it. The same tool serves
   the Native installer, which runs under macOS. An ADR records the DosType
   and the GUID.
3. **True boot on Hosted (S2).** A local AROS change, kept as a patch in this
   repository and never pushed, lets `hostdisk.device` take its unit pattern
   from a kernel argument instead of `/dev/disk%ld`. The boot module list
   gains hostdisk and the AFS+ handler. Proof: AROS boots with the image from
   2 as its only boot disk; the first command of the Startup-Sequence already
   runs from AFS+ (`AFSPlusInfo SYS:` answers), the desktop comes up, and the
   host checker finds the volume clean afterwards.
4. **Hardware CRC32C on AArch64.** The CRC instructions of ARMv8 replace the
   table on targets that have them; the table stays for the others. Proof:
   equality with the table over random buffers and lengths, and the hosted
   benchmark before and after.
5. **Driver contract.** What the handler requires of a block device (write
   ordering, flush, 64-bit commands, geometry, errors), written down, and a
   probe program a new driver must pass. For Native's USB and NVMe drivers.
6. **Apple Silicon profile.** Cache and commit defaults for machines with
   memory to spare; small machines keep the bounded ones.
7. **v2 tour.** An AROS program on the volume that shows clones, change
   watching, extended attributes and the health report, and snapshots where
   the volume offers them.
8. **Native under QEMU again.** Rebuild the Native tree and rerun
   `tools/check-macaros-native-block-qemu.sh` and its alpha-0 and replay modes
   on the current handler.

Each item ends with its own named check, a commit and a push.
