# AFS+ performance on AROS

> **ADRs:** [ADR-121](../adr/ADR-121-delayed-group-commit.md) · **Spec:** none ·
> **Tests:** [`bench-hosted-aros.sh`](../tools/bench-hosted-aros.sh) · **Milestones:** [roadmap-51](../implementation/milestones.md)

Where AFS+ stands against the Fast File System, on the one target where both
run side by side today, and what the comparison is worth. The lot-by-lot
account of how it was taken there is
[implementation/performance-program.md](../implementation/performance-program.md);
the format of the result bundle is
[benchmark-contract](../testing/benchmark-contract.md#native-aros-runner).

## What is measured

[`tools/bench-hosted-aros.sh`](../tools/bench-hosted-aros.sh) runs
`AFSPlusBench` twice in one boot of Hosted MacAROS: once on an AFS+ volume,
once on a Fast File System volume of the same size, same seed, same device
driver (`fdsk.device` over a host file). The workload is a source tree: 10
trees of 8 drawers of 32 files, 2,560 files of 6.3 MB in all, three in four
under 1 KiB. Five phases run over the whole tree before the next begins:
create, list, read with every byte compared, rename, delete.

Before the run the package is checked against its manifest and the AFS+ image
against the image checker; after it the image is checked again. Every file is
read back and compared. A result that is fast and wrong is a failure.

Each phase line carries what the phase cost the handler: library calls,
device flushes, block writes and cache reads, taken from
`afsplus_aros_counters` at the phase boundaries. Those counters are what the
work below was aimed at; the seconds follow them.

## Where it stands

One boot on Hosted MacAROS (Apple Silicon host, `darwin-aarch64`), 2026-09-20:

| Phase | AFS+ | Fast File System |
|---|---|---|
| create | 0.21 s | 0.40 s |
| list | 0.10 s | 0.08 s |
| read | 0.20 s | 0.34 s |
| rename | 0.22 s | 0.22 s |
| delete | 0.21 s | 0.09 s |
| **whole run** | **0.93 s** | **1.12 s** |

Per operation, AFS+: 8 library calls and 2.5 block writes for a create, 4
calls and 1.1 writes for a delete, and 30 device flushes for the 2,650
operations of the create phase, which are the commits of its windows.

AFS+ is ahead on create and read, level on rename, behind on list and delete.
Delete is the one phase with a gap worth naming: a Fast File System delete
unlinks a name and marks blocks free; an AFS+ delete stages the removal,
commits it with its window, and gives the blocks back in idle time, which is
what lets a cut lose nothing.

## How it got there

Every row is the same script, same workload, same seed, on the same machine:

| | AFS+ | FFS |
|---|---|---|
| first measurement | 55.8 s | 1.57 s |
| read cache | 35.5 s | 1.59 s |
| delayed group commit ([ADR-121](../adr/ADR-121-delayed-group-commit.md)) | 31.7 s | 1.53 s |
| hardware CRC32C, sliced table | 20.1 s | 1.66 s |
| decoded nodes and records kept beside cached blocks | 9.3 s | 1.46 s |
| idle-time reclaim of deleted files | 4.1 s | 1.59 s |
| a close is not a durability point | 2.4 s | 1.58 s |
| commit tree walk, allocator rover, one call per path | 1.9 s | 1.55 s |
| directories in the window, files that grow in place | 1.1 s | 1.15 s |
| inline keys, borrowed buffers, one encode per image | 0.93 s | 1.12 s |

The Fast File System moved as well, from 1.57 to 1.12 s. Part of that is
measurement spread, and part is a change to `dos.library`'s `Rename`, which
sent 28 packets for one rename and now sends 6
([AROS#1256](https://github.com/aros-development-team/AROS/pull/1256)); it
helps every file system on the machine. Both columns come from the same boot,
so the comparison holds at every row.

## What this proves, and what it does not

**Hosted measures software cost.** Storage is a file on the host's own disk
behind `fdsk.device`, and a device flush costs nothing there. That makes it
the right place to find the work the file system does to itself, which is
what every change above removed. It says nothing about a real medium.

**On hardware the ranking can change**, and in AFS+'s favour: the largest
single change above, a close that no longer commits, removed two device
flushes per created file. On Hosted that is free and shows as CPU time saved;
on an NVMe or a USB stick each one is a real barrier. Nothing here has run on
an M1 or a 68000 yet.

**One boot is one sample.** The numbers above are a single run each, not a
mean of many, and the spread between two runs of the same commit has reached
10 %. They are the right size for deciding what to work on next and the wrong
size for a claim about a percentage.

**The workload is one shape.** Small files in a deep tree, which is a source
tree and a system volume. Large sequential files, many readers at once and a
nearly full volume are separate measurements
([testing/performance-benchmarks.md](../testing/performance-benchmarks.md)).

## What is measured next

On hardware, when Macaros Native reaches a disk: the same script and workload
on an M1, where flushes cost what they cost, plus the boot time of a system
volume and the endurance run of
[aros-system-volume-qualification](../testing/aros-system-volume-qualification.md).

On the host meanwhile, the items the counters still show: the delete phase's
remaining gap, the per-batch allocation churn of a commit, and the block the
cache copies on every insert
([performance-programme](../implementation/performance-program.md)).
