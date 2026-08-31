# AROS system-volume qualification and AFS/AFS+ comparison

> **ADRs:** [ADR-046](../adr/ADR-046-hosted-aros-same-image.md),
> [ADR-047](../adr/ADR-047-hosted-aros-crash-replay.md),
> [ADR-048](../adr/ADR-048-hosted-aros-system-pivot.md),
> [ADR-049](../adr/ADR-049-hosted-aros-desktop-pivot.md) · **Spec:** none ·
> **Tests:** [benchmark-contract](benchmark-contract.md) · **Milestones:** M06, M08

The end state is not merely that AROS can access an AFS+ data volume. AROS must
be able to run its normal system tree from AFS+, and the resulting behavior must
be measured against the classic AFS/FFS family on equivalent paths.

<!-- toc -->

- [1. Qualification ladder](#1-qualification-ladder)
  - [S0: secondary same-image mount](#s0-secondary-same-image-mount)
  - [S1: post-bootstrap system-volume pivot](#s1-post-bootstrap-system-volume-pivot)
  - [S2: boot-selected AFS+ system volume](#s2-boot-selected-afs-system-volume)
  - [S3: recovery and repeated boot](#s3-recovery-and-repeated-boot)
- [2. What MacAROS performance proves](#2-what-macaros-performance-proves)
- [3. Comparison contract](#3-comparison-contract)
- [4. Mandatory system workloads](#4-mandatory-system-workloads)
- [5. Required measurements](#5-required-measurements)
- [6. Acceptance](#6-acceptance)
- [7. Qualified configurations](#7-qualified-configurations)

<!-- /toc -->

## 1. Qualification ladder

Each level proves a distinct property. Passing a later-looking synthetic test
does not waive an earlier correctness gate.

### S0: secondary same-image mount

MacAROS mounts the host-created Alpha-0 image through `fdsk.device`, runs the
create/read/write/truncate/rename/fsync probe, shuts down cleanly, and returns
the image to the host. The host adapter must read the target-created file and
the strict checker must report a clean volume.

This proves the native DOS packet, handler, block-device and on-disk paths. It
does not prove that AROS can use AFS+ as `SYS:`.

Hosted MacAROS S0 is qualified by [`tools/check-hosted-aros-alpha0.sh`](../tools/check-hosted-aros-alpha0.sh); ADR-046
records the runtime evidence and boundary fixes. Each later platform repeats S0
before advancing to its system-volume or performance gates.

The secondary-volume recovery extension is qualified by
[`tools/check-hosted-aros-crash-replay.sh`](../tools/check-hosted-aros-crash-replay.sh). It replays deterministic power-cut
images through the real Hosted handler; ADR-047 defines the modeled state space
and prevents this result from being confused with a physical power-cut claim.

### S1: post-bootstrap system-volume pivot

Start MacAROS through its existing boot path, mount an AFS+ image containing a
manifested copy of the AROS system tree, and switch the system assigns and path
to that volume. After the pivot, launch the desktop, preferences, command-line
tools and representative applications without reading executable or data files
from the old system tree except for explicitly recorded bootstrap components.

This is the first gate that supports the statement "AROS runs on AFS+" during a
normal session. It is still not an autonomous cold boot from AFS+.

S1 is implemented cumulatively. S1a proves the core assign pivot and execution
of a manifested command/library subset; [`tools/check-hosted-aros-s1.sh`](../tools/check-hosted-aros-s1.sh) passed
that gate and ADR-048 records its explicit bootstrap boundary. S1b adds the
desktop, preferences and representative applications.
[`tools/check-hosted-aros-s1b.sh`](../tools/check-hosted-aros-s1b.sh) has qualified S1b on Hosted MacAROS; ADR-049
records the payload, runtime evidence, case-policy boundary and exact claim.

### S2: boot-selected AFS+ system volume

Make the handler available before DOS chooses the boot volume, mark the AFS+
volume bootable with an explicit priority, and let the normal boot-volume
selection establish `SYS:` directly. The boot must not rely on finding
`afsplus-handler` inside the volume that requires that handler to be opened.

On MacAROS a file-backed transport may still be useful for repeatability, but
all bootstrap dependencies must be listed. A later physical-device run removes
the host-file transport from the claim.

### S3: recovery and repeated boot

Run clean shutdown, forced process termination and controlled power-cut cases
at every durability point used by the system-tree workloads. Each recovered
image must select an allowed state, replay within the bounded mount contract,
boot or fail safely, and pass the checker. Repeat cold boot/shutdown cycles to
catch resource and generation leaks.

## 2. What MacAROS performance proves

MacAROS on Apple Silicon is a significant benchmark for:

- filesystem-handler and DOS/VFS overhead in a real AROS execution;
- metadata algorithms, directory scaling, cache policy and transaction costs;
- application-visible latency for AROS tools and modern workloads;
- AFS/FFS versus AFS+ comparisons on one CPU, OS build and storage path; and
- exact block traffic, barriers, write amplification and recovery time when the
  device adapters are instrumented identically.

It is not by itself evidence for classic-machine speed. Native Apple Silicon,
large RAM, host file caching and an APFS-backed `fdsk.device` image do not model
a 680x0 CPU, a few MiB of memory, an IDE/CompactFlash controller or a real
rotating disk.

The project has three target platforms and four ordered validation stages.
Hosted MacAROS/macOS and native MacAROS/Apple Silicon are the first two target
platforms. The Amiga 500/m68k is the third; it is deliberately qualified in two
stages, emulator first and physical machine second. Results from the four
stages are published separately and never averaged:

1. Hosted MacAROS on macOS: modern AROS software cost and the primary rapid
   development regression gate;
2. native MacAROS on Apple Silicon: bare-metal integration without the Hosted
   transport or host filesystem in the execution path;
3. Amiga 500/m68k emulation: deterministic CPU, RAM and controller model for
   instrumentation, repeatability and crash testing before physical media; and
4. physical Amiga 500: named CPU, RAM, controller and storage medium.

Explicit cache budgets or CPU constraints may be applied inside a stage for
sensitivity analysis, but a constrained Hosted result is not promoted to an
emulated or physical-machine claim.

The project's first physical classic target is an Amiga 500. Its result bundle
must record the exact 680x0 CPU, Chip/Fast RAM, accelerator or expansion, ROM/OS
build, storage controller, medium and filesystem-handler versions. A stock and
an expanded A500 are distinct result configurations of the same target
platform.

The A500 qualification starts in the emulator with the bounded
reader/`NO_CHANGES` path, then a minimal read-write profile, deterministic
power cuts and timed workloads. Only the resulting qualified build moves to
disposable media on the physical machine. Unsupported optional features are
negotiated through the normal AFS+ profile/feature mechanism; the A500 does not
get a divergent disk format. Workload sizes are scaled to fit the recorded
machine, while operation mixes and durability points remain comparable to the
MacAROS runs.

## 3. Comparison contract

The baseline called "AFS" must identify its exact handler and on-disk variant;
where the platform calls the same family AFS or FFS, the result bundle records
both the binary version and DOS type. PFS3 and SFS may be additional references,
but must not silently replace the requested AFS/FFS comparison.

For a paired MacAROS result, keep constant where the filesystems permit it:

- MacAROS commit and boot tree;
- machine, power mode and background services;
- device adapter and backing medium;
- partition/image capacity and starting offset;
- operation trace, payload, seed and initial directory contents;
- warm/cold-cache state and run order; and
- durability level: buffered, file flush, or filesystem-wide flush.

If a classic filesystem cannot use the same block size or feature, report its
native recommended setup and the difference. Do not disable AFS+ durability to
make it resemble a weaker contract; publish both relaxed-throughput and equal-
durability comparisons when useful.

## 4. Mandatory system workloads

The system-volume suite includes:

- cold boot to shell-ready and desktop-ready markers;
- directory traversal and command lookup across `C:`, `L:`, `LIBS:` and
  `DEVS:`;
- launch and clean exit of a fixed command/application manifest;
- preferences write followed by immediate restart;
- copy, unpack and delete of a representative system update;
- compiler/build and Git-style temporary-file plus atomic-rename workloads;
- many-small-file create/stat/enumerate/delete;
- sequential and random file I/O; and
- recovery after interruption of the write-heavy workloads.

Every timed run first verifies the source manifest, and every write run ends
with filesystem-specific structural verification. Performance samples whose
correctness gate fails are invalid, not slow results.

## 5. Required measurements

In addition to [`testing/benchmark-contract.md`](benchmark-contract.md), native runs record:

- time to handler mount, log replay, shell-ready and desktop-ready;
- DOS packets by action and failures by error code;
- device reads, writes, bytes and `CMD_UPDATE` barriers;
- handler peak/steady memory and configured cache budget;
- host backing-file logical/physical growth for MacAROS image runs;
- AFS+ checkpoint, intent-log and reclaim counters; and
- all bootstrap reads that occur outside the selected system volume.

The virtual drive-activity API may drive a UI LED during interactive runs, but
benchmark collection uses counters or buffered trace records. UI callbacks and
animation are disabled for timing runs unless their cost is the subject of the
test.

## 6. Acceptance

"AROS runs on AFS+" is accepted at S1 and must always be qualified as a
post-bootstrap pivot until S2 passes. "AROS boots from AFS+" requires S2.
"AFS+ is faster than AFS/FFS" is never a global statement: it must name the
workload, platform class, durability policy and resource budget from the result
bundle.

## 7. Qualified configurations

This section records which configuration each gate proves; the one-line
milestone state is in [implementation/milestones.md](../implementation/milestones.md)
(M06, M08).

| Platform | Proven by | Claim |
|---|---|---|
| Hosted MacAROS on macOS | [`check-hosted-aros-alpha0.sh`](../tools/check-hosted-aros-alpha0.sh), [`check-hosted-aros-crash-replay.sh`](../tools/check-hosted-aros-crash-replay.sh), [`check-hosted-aros-s1.sh`](../tools/check-hosted-aros-s1.sh), [`check-hosted-aros-s1b.sh`](../tools/check-hosted-aros-s1b.sh) | S0 same-image round trip through real macFUSE, six intent-log replay cases, S1a core pivot and S1b desktop session ([ADR-046](../adr/ADR-046-hosted-aros-same-image.md)–[ADR-049](../adr/ADR-049-hosted-aros-desktop-pivot.md)) |
| Native MacAROS, Apple-AArch64 under QEMU | [`check-macaros-native-alpha0-qemu.sh`](../tools/check-macaros-native-alpha0-qemu.sh), [`check-macaros-native-replay-qemu.sh`](../tools/check-macaros-native-replay-qemu.sh) | S0 operation matrix and six replay cases through the retained-image RAM transport; pre-hardware only, no reset-durability claim ([ADR-053](../adr/ADR-053-native-macaros-retained-image-transport.md), [ADR-054](../adr/ADR-054-native-macaros-crash-replay-extraction.md)) |
| Native AROS/m68k, M68020-or-newer emulator profile | [`check-aros-m68k-alpha0-fsuae.sh`](../tools/check-aros-m68k-alpha0-fsuae.sh) | S0 operation matrix and six replay cases with the experimental Rust/m68k `std` static library ([ADR-055](../adr/ADR-055-aros-m68k-emulator-gate.md), [ADR-056](../adr/ADR-056-native-aros-m68k-alpha0-and-replay.md)) |
| Native AROS/m68k, A500-configured plain M68000 emulator profile | [`check-aros-m68k-alpha0-fsuae.sh`](../tools/check-aros-m68k-alpha0-fsuae.sh) | S0 and replay with M68020 long-multiply instructions rejected before boot; measured 8-MiB emulator profile with stable shutdown/restart cycles; 4 MiB does not boot the control profile ([ADR-057](../adr/ADR-057-plain-m68000-emulator-gate.md), [ADR-058](../adr/ADR-058-m68000-memory-and-restart-lifecycle.md)) |
| All of the above, as one result | [`check-mountable-alpha0.sh`](../tools/check-mountable-alpha0.sh) | Mountable Alpha-0 composite completion; no physical-hardware or format-freeze claim ([ADR-060](../adr/ADR-060-mountable-alpha0-completion-gate.md)) |

Open gates, in ladder order: S2 boot-selected volume and S3 repeated
boot/recovery on every platform; native MacAROS on Apple hardware with a
persistent reset-durable transport; the performance budget of section 2; the
physical Amiga 500. Every gate treats a guest failure requester as a verdict
([ADR-059](../adr/ADR-059-guest-failure-diagnostics-are-gate-verdicts.md)).
