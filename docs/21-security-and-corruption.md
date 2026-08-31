# 21. Security and Corruption Handling

> **ADRs:** none · **Spec:** none ·
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

## 3. Symlink loops

Path resolution has a bounded symlink-follow count and loop detection.

## 4. Tree corruption

B+ tree readers validate:

- level consistency
- ordering
- child block ranges
- key ranges
- checksums
- cycle absence

## 5. Resource exhaustion

An attacker-controlled volume must not force absurd allocations by declaring huge lengths or tree fanout.

Readers stream where possible and apply implementation-defined safe caps.

## 6. Fuzzing

All independent metadata decoders must have fuzz targets.

Fuzzing is a release requirement, not optional hardening after 1.0.
