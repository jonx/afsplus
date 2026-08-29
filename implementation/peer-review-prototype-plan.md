# Peer Review Follow-up and First Prototype Plan

Status: implementation guidance after external review

## Why this document exists

The project has reached the point where additional speculative specification has diminishing returns.

The next milestone is to make the core assumptions executable and use deterministic crash/resource tests to decide the remaining format questions.

Peer review identified several ambiguities. Some were objective inconsistencies and have been corrected directly in the specification. The major architecture questions below remain deliberately open and should be decided by prototypes/measurements rather than by adding more prose.

## Corrections already accepted

These do not require further architectural debate:

- timestamps are signed Unix-epoch seconds in UTC plus nanoseconds; host-local/Amiga time is adapter behavior
- original valid UTF-8 name bytes are preserved; normalization/casefold applies to a stored comparison key
- Unicode normalization/casefold table version is a volume format parameter
- B+ tree key ordering is binary over a frozen key encoding, never host locale collation
- catalog records are namespace-link records; hard-linked objects appear once per link and consumers deduplicate by object ID when needed
- catalog is non-authoritative/rebuildable/discardable
- change stream is non-authoritative/discardable but **not reconstructible history**; loss/reset means `RESCAN_REQUIRED`
- optional future user-data checksum association has a reserved feature identity and extent flag path without freezing the checksum design
- duplicate documentation numbering is removed
- the feature framework explicitly supports experimental -> stable -> deprecated -> retired lifecycle without reusing feature IDs or silently breaking old active volumes
- concurrency/iterator semantics are an explicit epoch-1 contract, not accidental implementation behavior

## Architecture blocker 1: committed data update policy

Question:

> When an application rewrites an already allocated unshared file range, does AFS+ overwrite that physical data in place or allocate new blocks until commit?

Prototype/measure:

- full data COW
- in-place overwrite for unshared data
- hybrid policy only if measurements justify the extra semantics

Measure:

- 4 KiB random rewrite workload
- database/VM-image style rewrite patterns
- reflink writes
- fragmentation
- CPU
- peak RAM
- physical bytes written
- crash states
- cost/feasibility of serving an exact previous committed content generation

Reflink-shared ranges always require COW regardless of the final general policy.

## Architecture blocker 2: fsync under global checkpoints

Start with the simplest checkpoint-COW engine.

Measure repeated:

```text
write small file
fsync
```

and Git/package-manager style create/rename/fsync workloads.

If checkpoint cost is unacceptable, prototype a **small durability/intent log in addition to checkpoint COW**.

Do not build a second complete redo-journal filesystem engine merely for theoretical comparison.

The disk layout reserves a discoverable extension point for such an auxiliary log without freezing its record format.

## Architecture blocker 3: free-space state under metadata COW

A flat authoritative bitmap copied through ordinary COW is self-referential because allocating its replacement modifies the bitmap itself.

Prototype candidates:

1. PFS3-like separately managed metadata reserve
2. bitmap + bounded transaction delta/log
3. spacemap/log-style authoritative allocation changes with condensation
4. small bootstrap/reserve allocator plus region structures

Required properties:

- no double allocation
- crash consistency
- bounded recovery
- bounded memory
- low-space operation
- repairability
- low write amplification

Do not freeze the allocation-region free-space encoding before this prototype.

## Architecture blocker 4: retained content generations

The developer/security API wants a race-free handle to an exact committed content generation.

That is a form of per-object historical retention and overlaps with snapshot-like mechanics.

The prototype must determine:

- how long an old generation can be retained
- what pins its extents
- what happens under storage pressure
- whether the guarantee is cheap only under full data COW
- whether the API is best-effort (`GENERATION_NOT_AVAILABLE`) rather than guaranteed retention

General user-visible snapshots remain a separate product feature/non-goal for the initial release.

## Architecture blocker 5: ACL scope

Do not make the full proposed NFSv4/Windows-like evaluation semantics an epoch-1 dependency before a real multi-user implementation consumes and tests them.

Near-term format goal:

- reserve/version a security-descriptor reference/blob container
- preserve unknown richer security metadata
- prevent simple hosts from silently destroying it

Then prototype/evaluate canonical ACL semantics with real POSIX/Windows adapters before declaring the evaluation model stable.

## First contributor implementation scope

The first useful contribution is host-side and independent of AROS/Macaros Native.

Suggested sequence:

### 1. Rust workspace

```text
crates/afsplus-format
crates/afsplus-block
crates/afsplus-core
crates/afsplus-check
```

### 2. Block backends

- memory backend
- sparse host-file backend
- trace wrapper
- deterministic fault wrapper
- power-cut wrapper

### 3. Smallest mountable image

- format/identification descriptor
- checkpoint slots A/B
- root object
- minimal metadata block encoding
- checker that validates the image

### 4. First writable transaction

- create one object
- allocate minimal metadata
- commit checkpoint
- remount
- verify

### 5. Crash matrix

Inject failure after every block write/flush in that operation and prove that mount returns either an allowed pre-commit or post-commit state.

### 6. Allocation experiment

Implement enough region allocation to expose and test the allocation-metadata recursion issue rather than hiding it behind a placeholder.

## Explicitly not first implementation work

Do not start the initial core implementation with:

- full ACL engine
- content inspection/antivirus integration
- global catalog
- change stream
- AtomicBatch
- tiny-file packing
- FUSE
- native AROS handler
- LLM-specific tuning

Those ideas remain design targets/proposals, but they should consume a proven core rather than define it prematurely.

## Decision discipline

A Proposed feature remains removable until it has:

1. a real consumer/use case
2. an implementation
3. measured resource cost
4. crash/recovery semantics
5. compatibility classification

Experiments may disappear completely.

Once a feature has produced active on-disk state in a released format, its identifier is permanent. Future implementations may deprecate/retire creation of that feature, but existing active volumes require retained support or explicit migration.

## Success criterion for the next phase

The next meaningful project milestone is not more documentation.

It is:

```text
format image
 -> mutate
 -> checkpoint
 -> kill power at every point
 -> remount
 -> verify exact allowed state
```

with CPU/RAM/I/O accounting from the beginning.
