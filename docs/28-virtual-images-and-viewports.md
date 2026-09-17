# 28. Virtual Images, Overlays, and Filesystem Viewports

> **ADRs:** none · **Spec:** none ·
> **Tests:** [image_diff](../crates/afsplus-check/tests/image_diff.rs) ·
> **Milestones:** roadmap-37

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
  - [The semantic image diff](#the-semantic-image-diff)
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

### The semantic image diff

[`afsplus_check::diff`](../crates/afsplus-check/src/diff.rs) compares the
committed states of two images, and `afsplus-image-diff` prints the result as
the short human form or, with `--json`, as the versioned machine-readable form
of [ADR-025](../adr/ADR-025-structured-management-api.md), at schema version 2. It reports objects
created and removed; names added, removed, retargeted and moved, where an
object whose single name disappeared on one side and appeared on the other is
one move rather than a creation and a removal; hard-link counts; type, size,
allocated size, protection bits, the three timestamps, content generation,
symlink target and the stored comment of
[ADR-106](../adr/ADR-106-stored-object-comment.md), where the empty comment is the
absent one, so setting, changing and clearing a comment are the same change
with different ends; the presence, format identity, version, length, divergence
mark and bytes of a security descriptor; the logical byte ranges whose content
changed, byte precise, where a byte past the end of a file is absent and an
absent byte differs from any present byte, so a truncation and an extension
are ranges like any other change; the allocation changes that change no
content, as mapped, shared and unwritten block counts, which is what a clone,
a shared range and a preallocation produce; and the volume facts, which are
the committed label of [ADR-104](../adr/ADR-104-volume-label-in-checkpoint.md),
the checkpoint generation, the free-block delta, the root and next object IDs,
the blocks and runs held by the reclaim queue, the entries of the orphan
directory and the snapshot registry. `--metadata` compares everything except
file content. Ordering is total: objects and orphans by object ID, names by
parent then name, snapshots by ID.

Two rules keep the answer simple. Content is compared by reading the bytes of
both files rather than by trusting equal physical mappings, because two images
are not necessarily copies of one another and the same block number on two
volumes is not the same data; the extent maps then serve for the allocation
summary rather than for the content answer. A single removed name and a single
added name for one object are reported as a move whatever the sequence of
operations between the two states, because the two committed states are all
the diff sees and they say the object kept one name in another place.

The block walk of [docs/26](26-debug-observability.md) attributes blocks and
says nothing about the fields of a record, so the comment, like every other
field, is the diff's own reading. The diff is its own reader, like that walk: it selects the checkpoint and descends
every tree with the block codecs of `afsplus-format` alone, sharing no
traversal, claim set or loader with the checker or with the core, so a
disagreement with either is a finding about one of them. Every tree is read
through a cursor that holds one leaf node and the pending child block numbers
of the path above it, so a directory, an object map or an extent map of any
size costs one block of items and `O(fan-out x depth)` block numbers per
image, and content is compared one logical block per side. What the diff
accumulates is the report, whose size is the number of changes rather than the
size of the volume. A structure that cannot be decoded is reported rather than
guessed: what a damaged tree hides stays uncompared, so a damaged object map
or directory does not turn into a volume of removals, and the command returns
its media status instead of "no difference".

[Its test](../crates/afsplus-check/tests/image_diff.rs) builds image pairs with
the real core, one per operation family, and requires the diff to be the
literal consequence of the operations performed. Each pair carries three
further witnesses: an image does not differ from itself, the reversed diff is
the mirror of the diff, and the reported byte ranges equal a brute-force
comparison of the two files read through the core.

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
