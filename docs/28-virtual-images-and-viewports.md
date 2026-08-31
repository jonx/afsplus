# 28. Virtual Images, Overlays, and Filesystem Viewports

> **ADRs:** none · **Spec:** none ·
> **Tests:** none · **Milestones:** none

<!-- toc -->

- [1. Core rule: an image is a real volume](#1-core-rule-an-image-is-a-real-volume)
- [2. Sparse files are ideal for development](#2-sparse-files-are-ideal-for-development)
- [3. Block-backend composition](#3-block-backend-composition)
- [4. SliceBackend: a true viewport into a larger disk image](#4-slicebackend-a-true-viewport-into-a-larger-disk-image)
- [5. OverlayBackend: disposable writable views](#5-overlaybackend-disposable-writable-views)
- [6. Forking a test state](#6-forking-a-test-state)
- [7. CheckpointView: logical historical viewport](#7-checkpointview-logical-historical-viewport)
- [8. Image files as first-class virtual disks on AROS](#8-image-files-as-first-class-virtual-disks-on-aros)
- [9. Nested images and reflinks](#9-nested-images-and-reflinks)
- [10. Directory projections are different](#10-directory-projections-are-different)
- [11. Debugging commands worth providing](#11-debugging-commands-worth-providing)
- [12. Guiding principle](#12-guiding-principle)

<!-- /toc -->

## 1. Core rule: an image is a real volume

The canonical development image is a raw sparse file containing exactly the bytes that would exist on a physical AFS+ volume.

Example:

```text
test.afsplus
```

It is not a special simulator format.

The same image can be:

- opened by host tools
- mounted through FUSE on macOS/Linux
- passed through the checker
- copied to a partition/device
- later mounted on AROS through a file-backed virtual block device

This guarantees that host testing exercises real AFS+ on-disk structures.

## 2. Sparse files are ideal for development

A host sparse file can advertise a large virtual capacity while consuming physical host storage only for written regions.

That allows testing 1 TB or multi-TB geometry, high object IDs, allocation-region boundaries, large free-space maps, and near-end-of-volume arithmetic without allocating the full image size.

## 3. Block-backend composition

AFS+ should treat the image file as one implementation of the block-device contract.

Useful backends/wrappers:

```text
MemoryBackend
FileBackend
SliceBackend
OverlayBackend
TraceBackend
FaultBackend
PowerCutBackend
ReadOnlyBackend
LatencyBackend
```

Wrappers compose:

```text
FaultBackend(
  TraceBackend(
    OverlayBackend(
      FileBackend("base.afsplus"),
      FileBackend("run.delta")
    )
  )
)
```

The filesystem core does not know whether it is running on NVMe, a normal file, RAM, an overlay, or a fault injector.

## 4. SliceBackend: a true viewport into a larger disk image

A common development case is a complete GPT disk image containing an AFS+ partition.

`SliceBackend` exposes only a byte/LBA range as a block device:

```text
whole-disk.img

+------------------+
| EFI              |
+------------------+
| Apple/bootstrap  |
+------------------+
| AFS+             | <---- SliceBackend(start, length)
+------------------+
```

The AFS+ core sees LBA 0 at the start of the slice.

This is useful for installer tests and prevents partition-offset arithmetic from leaking into filesystem code.

## 5. OverlayBackend: disposable writable views

For tests, repeatedly copying a multi-gigabyte image is wasteful.

`OverlayBackend` provides a writable view of an immutable base image:

```text
base.afsplus  (read only)
      ^
      |
   fallback
      |
run-123.delta (only changed blocks)
```

Read behavior:

```text
if block exists in delta:
    return delta block
else:
    return base block
```

Write behavior:

```text
write only to delta
```

Benefits:

- instant reset by deleting the delta
- cheap parallel test branches
- preserve the exact failing state
- compare two algorithms against the same base
- tiny artifacts when only a few metadata blocks changed
- deterministic fault-injection runs

The overlay format is a host/test facility and is not part of the AFS+ on-disk format.

## 6. Forking a test state

A test harness can create logical branches cheaply:

```text
base.afsplus
   |
   +-- alloc-test.delta
   +-- rename-test.delta
   +-- crash-001.delta
   +-- crash-002.delta
```

A failure artifact can therefore contain:

```text
base image ID/hash
small delta file
operation trace
fault configuration
flight recorder
expected/observed invariant report
```

That is much easier to archive and reproduce than copying a complete disk image after every test.

## 7. CheckpointView: logical historical viewport

AFS+'s checkpoint architecture creates another useful type of view.

If an older checkpoint generation is still retained and all of its referenced blocks remain quarantined, tooling can expose that generation as a read-only logical filesystem view:

```text
current generation 184
previous generation 183

mount generation 183 read-only for inspection
```

This is not the same as promising general user snapshots.

It is initially a developer/recovery feature for:

- comparing pre/post transaction state
- examining a corruption introduced by one generation
- verifying deferred reclamation
- inspecting what would have mounted after a crash at a specific commit boundary

The tool must refuse the view once required retired blocks are no longer retained.

## 8. Image files as first-class virtual disks on AROS

AROS should eventually have a generic file-backed block device, conceptually:

```text
filedisk.device
```

Then a normal file can be attached as a virtual disk and any compatible filesystem handler can mount it:

```text
AttachDisk Work:test.afsplus AS unit 3
Mount AFSPLUS: from filedisk.device unit 3
```

This should be generic infrastructure, not hard-coded into AFS+.

Benefits beyond AFS+:

- mount disk images without repartitioning hardware
- test exFAT/FFS/PFS images
- recovery work
- installer testing
- reproducible filesystem bugs
- nested development environments

## 9. Nested images and reflinks

Once AFS+ supports reflinks, a large virtual disk image stored inside AFS+ can itself be cloned cheaply:

```text
Work:VM/base.img
CloneFile -> Work:VM/test-a.img
CloneFile -> Work:VM/test-b.img
```

The host filesystem shares unchanged physical extents while the guest/image contents diverge independently.

This is especially attractive for OS and filesystem development workloads.

## 10. Directory projections are different

A host-directory projection that synthesizes a virtual filesystem directly from host files can be useful for integration tests, but it does not exercise the AFS+ on-disk format.

Therefore it must never replace raw-image tests.

If implemented, it belongs as a convenience/import adapter, not as a correctness backend.

## 11. Debugging commands worth providing

Potential host CLI:

```text
afsplus image create test.afsplus --size 64G --sparse
afsplus image mount test.afsplus ./mnt
afsplus image overlay base.afsplus run.delta
afsplus image fork base.afsplus test-001.delta
afsplus image inspect test.afsplus --checkpoint 184
afsplus image slice whole-disk.img --gpt-partition <uuid>
afsplus image replay base.afsplus trace.json fault.json
afsplus image diff before.afsplus after.afsplus --metadata
```

`image diff --metadata` should explain semantic differences rather than only listing changed blocks:

```text
checkpoint 183 -> 184
object 42 extent root changed
allocation region 7: +3 allocated, -1 retired
catalog generation advanced
reclaim queue +1 item
```

That becomes a powerful microscope for filesystem development.

## 12. Guiding principle

Virtualization should happen at clean boundaries:

```text
real AFS+ format
      |
block-device abstraction
      |
file / RAM / slice / overlay / physical disk
```

The test environment may be virtual; the filesystem semantics under test must be real.
