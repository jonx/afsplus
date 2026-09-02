# Explicit hybrid data-update policy

> **ADRs:** none · **Spec:** none ·
> **Tests:** [data-policy qualification](../testing/data-policy-qualification.md) ·
> **Milestones:** M03, M14

Target on acceptance: a numbered ADR amending the transaction and file-extent
design. Decisions requested: Q1-D1 through Q1-D4 below.

## Context and evidence

Open question Q1 asks whether private committed data uses full COW, in-place
overwrite, or a hybrid. The runtime-only prototype and its distinct crash
oracle are complete; results are retained in the
[bake-off report](../implementation/data-policy-bakeoff.md).

On 1,000 random 4 KiB rewrites, conservative private in-place update reduces
device writes from 26,972 to 8,000, allocations from 24,870 to 3,000,
retirements from 21,936 to 2,000, and final fragmentation from 1,767 extents
to one. A 32-page database hot set retains smaller but material gains. Append
and first writes to shared blocks remain COW and show no policy difference.

The cost is equally concrete: after an interrupted in-place transaction, an
older valid metadata checkpoint may expose old, new, or torn bytes in the
overwritten range. It cannot provide an exact historical byte generation.

## Proposed decision

Q1-D1: adopt an **explicit per-file hybrid**, not automatic workload
detection. Full data COW is the creation default and keeps the exact old/new
crash contract. An application or administrator may opt a file into the
weaker private-in-place contract through a filesystem-neutral policy API.

Q1-D2: in-place eligibility is fail-closed. Every touched block must be
materialized, mapped, and proven private; the write cannot extend the file.
Any shared marker, overlap uncertainty, hole, unwritten allocation or
extension makes the complete operation COW. Reflink-shared bytes always COW.

Q1-D3: the policy is persistent per file so remount and another implementation
cannot silently change its durability semantics. The exact object flag and API
encoding are separate format/API changes and must be proposed before the
runtime experiment becomes a shipping mode.

Q1-D4: exact-generation opens of an in-place file return
`GENERATION_NOT_AVAILABLE` once a later overwrite can have changed the same
physical data. They never return bytes while claiming an exact historical
generation. COW files retain the stronger contract while their generation is
retained.

## Rejected alternatives

- Full COW for every private write: preserves the strongest old-generation
  semantics, but the measured random-write amplification and fragmentation
  are unacceptable for an OS volume that targets databases and VM images.
- In-place overwrite for every private write: makes the weaker crash contract
  implicit and removes a useful default guarantee from source trees, package
  stores and content-inspection consumers.
- Automatic hot-file detection: changes durability semantics based on access
  history and would make recovery and cross-implementation behavior
  unpredictable. No measurement here justifies it.

## Required follow-up before format freeze

- propose and review the object flag plus filesystem-neutral policy API;
- reject policy changes while shared mappings cannot be safely privatized, or
  define their complete transition transaction;
- add exact-generation API tests for both policies;
- add low-space, multi-block/torn-write and injected-I/O-error matrices;
- measure process CPU/RSS and real storage separately from the memory backend;
- implement the same contract in the portable C path before epoch 1.
