# Proposed ADR: security preservation container

> **ADRs:** [ADR-031](../adr/ADR-031-portable-security-acls.md), [ADR-065](../adr/ADR-065-persistent-data-update-policy.md), [ADR-075](../adr/ADR-075-revocable-backup-capability.md), [ADR-082](../adr/ADR-082-backup-object-metadata.md) · **Spec:** [invariants](../spec/invariants.md) ·
> **Tests:** [security conformance](../testing/security-model-conformance.md) · **Milestones:** M14

Target on acceptance: a numbered ADR in `adr/` that closes ROADMAP B5 and the
format half of Q5 in [open questions](../implementation/open-questions.md);
the wire layout lands in [disk layout](../spec/disk-layout.md) and
[docs/04](../docs/04-object-model.md), the projection rule in
[docs/30](../docs/30-portable-security-model.md), and the feature identity in
the [feature registry](../spec/feature-registry.toml).

Decisions requested: M1 (container and reference), M2 (feature class), M3
(projection rule), M4 (lifetime and ownership), M5 (what stays outside
epoch 1). It builds on the
[exact admission of object records](adr-object-record-admission.md), whose
extension path it is the first to use.

<!-- toc -->

- [Context](#context)
- [Decision](#decision)
  - [M1. Container and reference](#m1-container-and-reference)
  - [M2. Feature class](#m2-feature-class)
  - [M3. Projection rule](#m3-projection-rule)
  - [M4. Lifetime and ownership](#m4-lifetime-and-ownership)
  - [M5. Outside epoch 1](#m5-outside-epoch-1)
- [Compatibility classification](#compatibility-classification)
- [Format-change procedure](#format-change-procedure)
- [API contract consequences](#api-contract-consequences)
- [Consequences](#consequences)
- [Open after this decision](#open-after-this-decision)

<!-- /toc -->

## Context

[ADR-031](../adr/ADR-031-portable-security-acls.md) separates two commitments:
a container that carries rich security metadata across hosts, needed before
epoch 1, and the evaluation semantics of a canonical ACL, which need real
POSIX and Windows adapters first. The epoch-1 freeze gate reads "unknown
security metadata can survive a simple-host round-trip without silent
downgrade" ([ROADMAP](../ROADMAP.md#epoch-1-freeze-gates)).

The object record holds one security field, the 32-bit protection word. Its
96-byte payload has no spare bytes, and every mutation path re-encodes the
decoded fields, so metadata a writer does not model is lost at the next
rewrite. A container therefore has to be a field of the record that every
epoch-1 implementation decodes, carries and re-encodes, while the bytes it
points at stay opaque.

## Decision

### M1. Container and reference

A security descriptor is an opaque byte string of 1 to 65,536 bytes, tagged
with a 32-bit format identity and a 16-bit format version. The filesystem
stores it, returns it and removes it. It never parses it, and no mount,
allocation, lookup or repair decision reads it.

Object flag bit 2, `OBJECT_FLAG_SECURITY_REF`, extends the fixed object
payload from 96 to 112 bytes. The 16 added bytes are the security reference,
placed before any inline symlink target:

```text
offset size field
96     8    first descriptor segment LBA (nonzero)
104    4    descriptor length in bytes (1..=65536)
108    2    segment count (the ceiling of length / segment capacity)
110    2    reference flags: bit 0 projection diverged; other bits zero
```

The descriptor occupies a chain of `"AFSX"` blocks. Each is a checksummed
metadata block whose header owner is the object ID:

```text
offset size field
0      4    descriptor format identity (nonzero)
4      2    descriptor format version
6      2    reserved (zero)
8      4    total descriptor length
12     2    segment index
14     2    segment count
16     8    next segment LBA (zero on the last segment)
24     n    descriptor bytes
```

Every segment except the last is full (block size minus 56 bytes: 4,040 at
4 KiB, so the bound is 17 segments). Admission is exact for the reference and
the segment alike: zero header flags, exact payload length, zero tail, zero
reserved field, flag and field present together, identity and lengths equal
along the chain. Files, directories, inline symlinks and the root directory
carry a reference; the internal orphan directory does not. A symlink with a
reference has 16 fewer bytes for its target.

Format identities are registry values ([feature framework](../docs/09-feature-framework.md)).
The identity names the encoding of the bytes (a canonical AFS+ descriptor, a
POSIX ACL set, a Windows self-relative security descriptor, a host-private
range); the container treats every nonzero value alike. One descriptor per
object: a host that needs several encodings defines an envelope format.

### M2. Feature class

The volume identity `org.aros.afsplus:security-descriptors` is INCOMPAT bit 3.
An implementation without the container cannot decode a flagged record
(exact admission refuses the unknown flag and length), so it cannot list the
directory that holds it, and a writer that skipped the reference would drop it.
RO_COMPAT would promise a read-only mount that fails on the first flagged
object. Identification is immutable, so the formatter sets the bit per
profile; the reference on a volume without the bit is corruption.

Every epoch-1 implementation implements the container. The bit exists so
that images, fixtures and corpus entries written before this decision stay
valid, and so that the constrained profiles can format volumes that never
carry descriptors.

### M3. Projection rule

The protection word stays the classic projection: the field a simple host
reads, shows and enforces. The descriptor never changes when the protection
word changes, and no ordinary operation discards descriptor bytes of a live
object. An edit of the protection word on an object that carries a
descriptor follows the host's projection policy:

- **strict** (the default at every mount): the edit is refused with a
  distinct error and nothing changes. A host that does not evaluate the
  descriptor cannot show that the new value agrees with it.
- **preserve**: the edit is applied, every descriptor byte stays, and
  reference flag bit 0, projection diverged, is set in the same transaction.
  A host that evaluates the format sees that the two views disagree and
  reconciles them.

The rule covers the protection setter and exact metadata restoration alike,
and applies only when the value changes. Setting a descriptor clears the
mark, because the caller supplies the descriptor that matches the object.
Clearing a descriptor is a separate, explicit operation: it is the downgrade
that [docs/30](../docs/30-portable-security-model.md) requires to be requested,
never implied. The policy is host runtime state and is absent from the disk.

### M4. Lifetime and ownership

- A chain has exactly one owner, the object whose record references it.
  Rewrites of the record carry the reference unchanged; segments are
  immutable, and replacement writes a new chain and retires the old one in
  one transaction.
- Rename, move, hard link, data write, truncation, policy change, timestamp
  restoration and orphaning keep the reference. Deleting the final link,
  through the direct path, the batch and window engine, atomic replacement
  or orphan cleanup, retires the chain with the record.
- No chain is ever shared. `CloneFile` copies one: the destination receives
  its own segments, allocated and published by the clone transaction, with
  the source's format identity, version, bytes and divergence mark, so a
  crash shows no clone or a clone with its complete descriptor. `CloneRange`
  is a content write and keeps the destination's own
  ([clone metadata inheritance](adr-clone-metadata-inheritance.md)).
- Retiring a chain, whether the object dies or its descriptor is replaced,
  proves one segment at a time: valid magic and checksum, this owner, the
  expected position, the reference's identity and length, and a committed
  generation, from the first segment up to the first invalid link. Only
  proven segments are freed, so a damaged chain never makes its object
  undeletable, and no block that was not proven to belong to the chain is
  ever handed back. Freeing an unproven block and leaking one are both
  defensible, and the leak is the safer answer: a wrongly freed block can be
  reused by another object and destroy live data, while a leaked block costs
  space the checker can name. Reading a damaged descriptor still fails:
  descriptor bytes are returned whole or not at all.
- The checker claims every segment in the single ownership set, so a leaked,
  doubly referenced, foreign or damaged segment is a finding, and it reports
  the flag on a volume without the feature as corruption. The remainder of a
  partially retired chain appears there as a block owned by nothing, which
  is the existing leak finding and needs nothing new on the wire.
- The volume combination with persistent snapshots
  ([ADR-070](../adr/ADR-070-persistent-snapshot-priority.md)) is refused at
  mount and is absent from the formatter: historical ownership of descriptor
  segments under the lifetime ledger is unqualified.

### M5. Outside epoch 1

The base writable milestone defines no principal encoding, no ACE layout, no
inheritance, no evaluation order, no audit semantics and no shared descriptor
store. Those belong to descriptor formats, each behind its own format
identity, validated by the adapters that
[ADR-031](../adr/ADR-031-portable-security-acls.md) names. Shared immutable
descriptors ([docs/30 section 8](../docs/30-portable-security-model.md#8-security-descriptors-as-shared-objects))
stay an optimisation candidate: the reference has room for it through a new
reference flag and an ownership rule of its own.

## Compatibility classification

INCOMPAT, bit 3, permanent identity `org.aros.afsplus:security-descriptors`.
Volumes without the bit are byte-identical to volumes written before this
decision. Object flag bit 2 and block type `"AFSX"` are allocated and never
reused.

## Format-change procedure

| Step | State |
|---|---|
| Specification update | Layouts above land in the disk layout and docs/04 on acceptance; `crates/afsplus-format/src/security.rs` and `object.rs` carry them as module documentation today |
| ADR | This proposal |
| Compatibility classification | INCOMPAT bit 3, reasoned in M2 |
| Conformance image | Open: one image per object type with a multi-segment descriptor, plus resealed negative images, joins the shared corpus with the portable C lot |
| Parser tests | `crates/afsplus-format/tests/security_container.rs`: literal wire bytes of the reference and the segment, flag and field congruence in both directions, five malformed references, directory and symlink carriers with the 16-byte target reduction, literal segment geometry, seven invalid segments and four resealed corruptions. The independent fuzz oracle (`fuzz/src/object_payload.rs`) models the reference separately from the decoder |
| Repair-tool behavior | The checker validates and claims every chain and never edits one: descriptor bytes have an owner the tool does not understand ([tool rule](../spec/compatibility-rules.md#tool-rule)). Salvage of an object whose chain is damaged reports the loss of the descriptor explicitly |
| Resource impact | No cost on objects without a descriptor: the record stays 96 bytes and no path reads a segment. With one: 16 bytes in the record; one block per 4,040 descriptor bytes; one extra block read per segment on descriptor read, replacement and deletion, none on lookup, stat, data I/O or mount. Bounded at 17 segments by the 64 KiB limit. Peak memory is one descriptor |

Executable proof, in `crates/afsplus-check/tests/security_container.rs`:
descriptors of 1, 4,040, 4,041 and 65,536 bytes round-trip across remount; a
descriptor of a format nothing evaluates survives data writes, extent-tree
promotion, truncation, the data-policy flag, hard link, rename, timestamp
restoration, clone of its object, a batch unlink, child creation in its
directory and a symlink rename, byte for byte; strict refusal leaves the
protection word, the change time and the descriptor untouched; preserve keeps
the bytes and makes the divergence durable; every removal path, orphan
cleanup and intent-log replay retire the chain; three resealed segment
corruptions are checker errors; removing the retirement call makes the
checker report each leaked block; a chain whose first, middle or last
segment is resealed under a foreign owner still unlinks, still empties its
name, frees exactly the proven segments and leaves exactly the unproven ones
as leak findings, and the same holds for directory removal, a final unlink
inside a window, orphan cleanup, descriptor replacement and the explicit
clear; and 2,334 modeled power cuts over attach,
replace, preserve-mode edit and delete each mount to the state before or
after the transition with a clean checker verdict.

## API contract consequences

These belong to the Stage C API and handler work; this proposal changes no
API document.

- Filesystem API v2 gains three capability-gated operations: read, set and
  clear a security descriptor (format identity, version, opaque bytes), and
  the security fidelity query of
  [docs/30 section 17](../docs/30-portable-security-model.md#17-security-fidelity-must-be-queryable)
  reports whether the volume carries the feature and which projection policy
  the mount uses.
- The protection setter has a new failure: the edit is refused under the
  strict policy when the object carries a descriptor. The core reports
  `SecurityProjectionRefused`; the VFS layer maps it to `NotSupported` until
  the API assigns a distinct code. An AROS handler chooses its policy at
  mount and defaults to preserve: `ACTION_SET_PROTECT` on such an object
  applies the new protection word, keeps every descriptor byte and marks the
  projection as diverged, and the mount flag
  `AFSPLUS_AROS_MOUNT_FLAG_STRICT_SECURITY_PROJECTION` selects the refusal
  instead. Refusing and preserving are both non-destructive, and preserve is
  the better default for a classic host, because a refusal leaves the DOS
  user with no way to change protection at all while preserve keeps the
  bytes and records that the two views disagree. The core default stays
  strict, so a caller that never chooses gets the refusal. Metadata that the
  container does not hold keeps its own rule, the explicit downgrade request,
  because there is nothing on disk to preserve or to mark. Volumes without
  descriptors behave as before.
- Backup and restore transport the descriptor as one opaque value pair per
  object ([ADR-084](../adr/ADR-084-opaque-backup-value-pairs.md)); restoration
  sets the descriptor after the protection word, which also leaves the
  divergence mark clear.

## Consequences

- The freeze gate "unknown security metadata can survive a simple-host
  round-trip without silent downgrade" has an executable proof in the Rust
  core.
- A classic host needs no account service and no ACL code: it carries 16
  bytes and refuses or marks one kind of edit.
- Rich semantics evolve by format identity, without an epoch change and
  without touching the object record again.
- Volumes that enable the feature are unreadable by pre-decision prototypes
  and, until the parity lot, by the portable C reader, which fails closed on
  the unknown flag.

## Open after this decision

- Portable C reader and writer parity and the conformance images: the C
  object decoder has no single-test harness outside the portable-C suite,
  and that work is scheduled with the C portability lot.
- The registry of format identities, including the host-private range: no
  adapter exists, so values would be invented.
- Descriptors on volumes with persistent snapshots, and the second half of
  Q5 (who reads a historical snapshot after live permissions change): both
  need the snapshot lifetime ledger to own descriptor segments and a
  consumer that evaluates a descriptor.
- The canonical descriptor format of docs/30 (principals, ACEs, inheritance,
  audit): needs the POSIX and Windows adapters of ADR-031.
- Backup and restore wiring of the opaque pair: it belongs to the archive
  consumer of M13.
- Inheritance of a parent directory's descriptor at object creation: an
  evaluation-format question; the container creates every object without a
  descriptor.
