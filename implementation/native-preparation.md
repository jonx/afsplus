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

## State on 2026-09-19

| Item | State | Evidence |
|---|---|---|
| 1 | done, 9aadb07 | [`check-hosted-aros-resident.sh`](../tools/check-hosted-aros-resident.sh) |
| 2 | done, 0d8f159 | [ADR-122](../adr/ADR-122-aros-partition-identity.md); `sgdisk -v` finds no problem |
| 3 | done, 8acbe8c and 7bdcc30 | [`check-hosted-aros-s2.sh`](../tools/check-hosted-aros-s2.sh); the handler needs no C library to start |
| 4 | done, da0c61a | 8.86 against 1.75 GB/s on the host |
| 5 | done, f705a93 | [`check-hosted-aros-driver.sh`](../tools/check-hosted-aros-driver.sh); fdsk and hostdisk fail the barrier |
| 6 | done, 3db8c65 | `CACHE=AUTO`; at 1e45a23, clean: 3.15 s with 64 blocks in 256 MiB, 3.13 s with 1,028 blocks in 1 GiB (FFS 1.43 and 1.42 s) |
| 7 | done, 6aa5469 and 1e45a23 | AFSPlusTour runs in the S2 boot; the gate fails on any tour FAIL (shown with the tour pointed at `RAM:`) |
| 8 | waiting | the Native build tree is not on this machine and needs some 15 GiB this disk does not have to spare |

What Macaros Native takes from here: the handler, partition.library and its
disk device as boot modules; a GPT partition of type
`4146532B-BB67-46C5-AA4A-F502CA018E5E` written by `afsplus-disk wrap`, with
`AROS.boot` naming the CPU at its root; drivers that pass
`AFSPlusDriverProbe`; and the handler built with `+crc`.
