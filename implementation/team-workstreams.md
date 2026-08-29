# Team Workstreams

Work can proceed in parallel after the reader subset is frozen.

## Workstream A: format/core

Owns:
- encoding
- object model
- trees
- extents
- allocation
- transactions

## Workstream B: AROS integration

Owns:
- handler lifecycle
- DOS compatibility
- Filesystem API v2
- notifications
- namespace/path integration

## Workstream C: tools/repair

Owns:
- formatter
- inspector
- checker
- repair
- resize

Must use shared core code.

## Workstream D: catalog/indexing

Owns:
- global catalog
- change stream
- streaming enumeration
- Ferail benchmarks

## Workstream E: portability

Owns:
- host-file backend
- FUSE
- classic reader profile
- third-party integration examples

## Workstream F: validation

Owns:
- fuzzing
- crash injection
- property tests
- corruption corpus
- performance suite
- application qualification

## Merge rule

No workstream may introduce a new on-disk semantic without:
1. specification change
2. ADR or existing ADR reference
3. feature compatibility classification
4. conformance test
