# Roadmap

## Stage 0: Amiga-native design review

- review PFS3 source subsystem by subsystem
- document PFS3 atomic commit
- evaluate PFS4 B+ tree, tiny-file, and fragmentation ideas
- produce adopt/adapt/reject matrix
- revise AFS+ transaction and small-file ADRs before format freeze

## Stage A: make the specification executable

- freeze reader subset
- create binary encoder/decoder tests
- create conformance images
- portable reader
- fuzzing

## Stage B: make images mutable

- formatter
- allocator
- object mutation
- journal
- crash injection
- checker

## Stage C: integrate AROS

- handler
- DOS compatibility
- Filesystem API v2
- modern path semantics
- notifications

## Stage D: make it portable and pleasant

- FUSE
- third-party probe kit
- compatibility profiles
- classic reader

## Stage E: modern accelerators

- global catalog
- change stream
- fast enumeration API

## Stage F: production

- resize
- scrub
- performance qualification
- real SSD qualification
- independent format review
- epoch 1 freeze
