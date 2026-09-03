# 16. Classic and Constrained Systems

> **ADRs:** none · **Spec:** none ·
> **Tests:** [conformance](../testing/conformance.md), [fuzzing](../testing/fuzzing.md) · **Milestones:** M01, M12

## 1. Policy

Full feature parity on vintage systems is not a requirement.

The format must nevertheless avoid architectural choices that make a lightweight implementation impossible.

## 2. Profiles

### Reader profile

Must be practical with very little memory.

Supports:

- superblock
- object lookup
- directory iteration
- extent reads
- core metadata
- feature negotiation

No write support required.

### Classic RW profile

Adds:

- allocation
- object mutation
- rename/delete
- journal transactions
- normal metadata writes

Optional workstation features may be absent.

## 3. 64-bit arithmetic

64-bit on-disk values are not considered too heavy for 32-bit CPUs.

Implementations may use software helpers or pairs of 32-bit operations.

The benefits of avoiding permanent 2 GiB/4 GiB format limits outweigh the small CPU cost.

## 4. Memory budget

Core algorithms must process metadata incrementally.

Allocation regions are designed so a free-space bitmap can be tens of KiB rather than proportional to total disk size.

B+ tree traversal uses a bounded path stack.

Catalog iteration streams records.

Orphan cleanup reads the extent-tree tail by subtree ordinal and removes no
more than the configured logical-extent budget in one maintenance call. A
classic adapter can choose a small budget without changing the disk format;
modern hosts can choose a larger one.

## 5. Unsupported features

Classic implementations obey feature classes:

- ignore safe COMPAT features
- mount read-only for unknown RO_COMPAT
- refuse unknown INCOMPAT

## 6. Same format

There is no separate "AFS+ Classic" disk format.

Different implementations support different profiles of the same specification.
