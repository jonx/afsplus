# ADR-114: The common header's flags and owner belong to no kind that does not name them

Status: Accepted
Amends: ADR-113

## Context

[ADR-113](ADR-113-checkpoint-flags-word.md) closed the checkpoint's reserved
word and left the same question open for the five kinds that had no exact
admission rule: the identification block, the tree nodes, the bitmap pages,
the region descriptors and the intent-log records.
[ADR-112](ADR-112-block-zero-tail.md) had already taken the bytes after the
payload, for every kind at once. What remained is inside the payload and
inside the common header.

A survey answered it, before any decoder changed. Its probes reseal a valid
block with a valid checksum and a zero tail, so ADR-112 does not decide them.
Eight findings over four kinds, and one of them a divergence: the portable C
reader refuses a nonzero flags word on a bitmap page and on a region
descriptor, where the core admits it. It is the fourth time a second reader
has disagreed with the core about admission, which is what a second reader is
for, and the reason the survey is kept as a test that asks both readers rather
than one.

The identification block also admitted a payload longer than the layout of
the version it states, with nonzero bytes sitting past the last field. That is
the shape of the checkpoint defect of [ADR-111](ADR-111-checkpoint-zero-tail.md):
bytes inside the payload that belong to no field, which a rewrite would drop
and two readers may read differently.

## Decision

1. The common header's `flags` word is zero on every one of the five kinds. A
   nonzero word is corrupt, in every reader.
2. The common header's `owner` is zero on the identification block and on the
   intent-log record, which belong to the volume and never name an object.
   The tree node, the bitmap page and the region descriptor already give the
   field a meaning and already check it.
3. The identification payload is exactly the layout of the version it states.
   A payload longer than that version's layout is corrupt, whatever the extra
   bytes hold.
4. A later identification layout arrives as a new version in the version
   field, with its own exact length, not as extra bytes appended to the
   payload of a version already defined. A reader that meets a version it does
   not know refuses the volume, as it does today; it must not read the prefix
   it recognises and ignore the rest, because it cannot know whether the
   unknown suffix changes the meaning of that prefix.

## What each kind owed, and to which decision

| Kind | ADR-112 (the tail) | This ADR adds |
|---|---|---|
| Identification (`"AFSI"`) | bytes after `payload_len` | zero flags, zero owner, a payload exactly its version's layout |
| Tree node (`"AFST"`) | bytes after `payload_len` | nothing: it already checks flags, the owner against the tree's owner, its two reserved bytes and its length |
| Bitmap page (`"AFSB"`) | bytes after `payload_len` | zero flags in the core; the portable C reader already refused them. Its length is exact and its owner is the region |
| Region descriptor (`"AFSG"`) | bytes after `payload_len` | zero flags in the core; the portable C reader already refused them. Its length is exact and its owner is the region |
| Intent-log record (`"AFSJ"`) | bytes after `payload_len` | zero flags, zero owner. Its length is already exact through its operation parsing |

## Compatibility classification

Whole-format change without a feature identity: no image of this filesystem
has been released. Every encoder writes zero into both header fields and the
exact payload of the current identification version, so no written block is
refused and no written byte changes.

## Format-change procedure

| Step | State |
|---|---|
| Specification update | [Structure headers](../docs/03-on-disk-format.md#7-structure-headers) |
| ADR | This ADR |
| Compatibility classification | Whole-format change that narrows admission only, reasoned above |
| Conformance image | None among written images; the test builds the refused ones |
| Parser tests | [`reserved_fields.rs`](../crates/afsplus-check/tests/reserved_fields.rs): on one volume that holds a block of each of the five kinds, nine probes, each resealed with a valid checksum and a zero tail, each asked of the core and of the portable C reader through the entry point that reaches that kind (`afspr_probe`, `afspr_lookup_object`, `afspr_scan_intent_log`, and the writer's allocation search for the bitmap page and the region descriptor). The live bitmap page and region descriptor are chosen by their explain role, since only the bound slot of three is read |
| Repair-tool behavior | The checker reads through the shared decoders and reports such a block as corrupt; it never edits one |
| Resource impact | Two integer comparisons per decoded block |

No existing assertion moved: every test of `afsplus-format`, the fuzz unit
tests, the Rust codec fuzz gate (4,096 runs per target) and the portable C
reader gate pass unchanged.

## Consequences

- A divergence of this shape now fails a test instead of waiting for a survey:
  the probes ask both readers.
- The five kinds ADR-113 left to the M14 review are decided. What that review
  still owns for them is the byte offsets themselves, not their reserved
  fields.
