# Application Qualification

> **ADRs:** none · **Spec:** none ·
> **Tests:** none · **Milestones:** M13

## Rust/Cargo

Pass when:
- native toolchain can create and build representative projects on AFS+
- builds survive restart and cleanup
- >4 GiB artifacts can be addressed
- temporary/atomic replacement behavior is correct

## Git

Pass when:
- clone/checkout/status/commit/reset work
- symlinks behave correctly
- case-policy edge cases are documented
- lock-file durability behavior is correct

## Zed-style editor workload

Pass when:
- large project opens without pathological rescans
- rename preserves identity
- watch stream reports expected changes
- restart can catch up through persistent changes where feature is active

## Ferail

Pass when:
- full enumeration uses semantic API
- catalog acceleration is transparent
- catalog absence falls back correctly
- incremental indexing uses change stream
- expired change history triggers full rescan

## Moonstone

Define a representative project and record filesystem assumptions before qualification.
