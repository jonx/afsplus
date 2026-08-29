# Crash and Power-Failure Testing

## Goal

Prove that every acknowledged transaction recovers to a valid state and every unacknowledged transaction resolves to one of the explicitly allowed states.

The suite is informed by real failure modes found in PFS3/pfs3aio, not only synthetic generic crashes.

## Fault model

The block backend supports deterministic injection of:

- drop write N
- tear write N at sector/sub-block boundary
- short write
- read error for block N
- reorder writes where barriers do not forbid it
- fail flush/barrier N
- out-of-memory at allocation N
- crash immediately before/after each write or flush

## Core transaction workloads

- create
- mkdir
- append
- overwrite
- truncate
- rename same directory
- rename across directories
- atomic replace
- hard link
- unlink open file
- symlink
- xattr update
- allocation-region transition
- catalog update/rebuild
- change-stream append

## Checkpoint-specific failure matrix

Inject failure:

1. before any new metadata write
2. during leaf COW metadata write
3. during parent/root COW metadata write
4. during pre-checkpoint flush
5. during/torn alternate checkpoint write
6. during post-checkpoint flush
7. immediately after durable checkpoint
8. during later reclamation of retired blocks

Expected rule: mount chooses either the old valid checkpoint or the new valid checkpoint, never a hybrid authoritative tree.

## Allocation safety regressions

Mandatory tests derived from pfs3aio failure classes:

### Unknown allocator metadata

Make one allocation bitmap/region page unreadable. The allocator must not hand out any block whose state is unknown.

### Deferred-free resource exhaustion

Force allocation failure while recording retired/deferred-free extents. The filesystem may leak/quarantine capacity but must not reuse potentially reachable storage.

### Tiny cache eviction pressure

Run allocator, rename, extent-tree, and cleanup workloads with the smallest supported metadata cache. Force cache misses at nested call boundaries. Pinned pages must not be evicted and stale handles must be detected in debug builds.

### Rename ENOSPC/failure at every step

For cross-directory rename, inject failure at every allocation/write step. Result must be either:

- source still present and destination absent
- committed destination with source removed

Never orphan the object and never expose two accidental independent objects.

### Geometry and arithmetic

Fuzz:

- block sizes
- sector sizes
- partition start/end
- volume sizes near powers of two
- maximum 64-bit offsets

All arithmetic is checked before conversion/multiplication.

## Deferred-reclamation tests

Delete/truncate very large synthetic objects, crash after every reclamation batch, and verify:

- namespace state remains committed
- no live extent becomes free
- progress can resume
- repeating a completed batch is safe or detected
- pending space eventually returns to free pool

## Validation after every crash

1. open in NO_CHANGES mode
2. enumerate/validate checkpoint candidates
3. select expected recovery state
4. run full invariant checker
5. verify reachable file contents for acknowledged durability points
6. verify no physical block has conflicting live ownership
7. verify quarantined blocks are never in allocatable free space

A result that requires ordinary fsck repair after every injected crash does not satisfy the normal transaction guarantee.
