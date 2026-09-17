# 21. Security and Corruption Handling

> **ADRs:** [ADR-100](../adr/ADR-100-exact-object-record-admission.md), [ADR-110](../adr/ADR-110-exact-reclaim-admission.md) to [ADR-115](../adr/ADR-115-retire-unwritten-surface.md) ·
> **Spec:** [disk layout](../spec/disk-layout.md) ·
> **Tests:** [conformance](../testing/conformance.md),
> [developer-harness](../testing/developer-harness.md) · **Milestones:** M05

## 1. Treat disk data as untrusted

Every parser must validate:

- sizes
- offsets
- block ranges
- object IDs
- tree depth
- record counts
- UTF-8
- feature IDs
- checksum results
- arithmetic overflow

A corrupt volume must not cause out-of-bounds memory access.

## 2. Bounds-first parsing

Never calculate an address and then validate it.

Validate ranges and checked arithmetic before dereferencing or allocating.

## 3. Exact admission: one image per value

Bounds-first parsing keeps a reader safe. Exact admission keeps two readers
in agreement, which is a different property and equally required here: a byte
a reader accepts without a field is a byte the next rewrite drops and a second
implementation may read differently.

A structure is admitted only in its canonical image:

- **Zero tail.** Nothing follows the payload. The common header states the
  payload length, and the header verification of every reader refuses a
  nonzero byte after it, for every block kind
  ([ADR-112](../adr/ADR-112-block-zero-tail.md)).
- **Exact length.** A structure whose payload is a fixed layout is admitted at
  exactly that length: the object record with each optional field its flags
  announce ([ADR-100](../adr/ADR-100-exact-object-record-admission.md)), the
  reclaim root with its three areas
  ([ADR-110](../adr/ADR-110-exact-reclaim-admission.md)), the checkpoint at 168
  or 184 bytes ([ADR-111](../adr/ADR-111-checkpoint-zero-tail.md)), the
  identification block at the length of the version it states
  ([ADR-114](../adr/ADR-114-reserved-header-fields.md)).
- **Reserved fields are zero.** The common header's flags, and its owner on the
  kinds that belong to the volume rather than to an object; the reserved bytes
  inside a payload; the unused slots of a reclaim root's areas; the
  checkpoint's reserved word ([ADR-113](../adr/ADR-113-checkpoint-flags-word.md),
  [ADR-114](../adr/ADR-114-reserved-header-fields.md)).
- **Validated namespaces.** An object flag, an extent flag or a block magic
  outside the assigned set makes the structure corrupt. A reader does not skip
  what it does not know.
- **Retired surface is refused, not ignored.** A version or block kind this
  format no longer defines is corrupt, and its number is never reused
  ([ADR-115](../adr/ADR-115-retire-unwritten-surface.md)).

A reader written to be tolerant here is not being generous; it is admitting
images no writer produces, which is where two implementations drift apart and
where a crafted image finds room.

## 4. Symlink loops

Path resolution has a bounded symlink-follow count and loop detection.

## 5. Tree corruption

B+ tree readers validate:

- level consistency
- ordering
- child block ranges
- key ranges
- checksums
- cycle absence

## 6. Resource exhaustion

An attacker-controlled volume must not force absurd allocations by declaring huge lengths or tree fanout.

Readers stream where possible and apply implementation-defined safe caps.

## 7. Fuzzing

All independent metadata decoders must have fuzz targets.

Fuzzing is a release requirement, not optional hardening after 1.0.
