# ADR-023: Developer observability is a first-class requirement

Status: Accepted

## Context

Filesystem defects often corrupt the evidence required to diagnose them. Mature filesystems have accumulated tracing, fault injection, checkers, scrubbers, and specialist debugging tools over many years.

AFS+ is being designed from scratch and can avoid treating diagnosability as an afterthought.

## Decision

The portable core must provide structured observability and test hooks before the writable on-disk format is frozen.

Required architecture includes:

- operation and transaction IDs
- low-overhead structured flight recorder
- trace categories
- live trace sink callback
- semantic explain APIs
- deterministic test controls
- stable named fault-injection points
- power-cut capable block backend
- semantic operation record/replay
- selectable invariant-check levels
- deliberately tiny-cache test mode

Most facilities are runtime-optional and must impose near-zero cost when disabled.

## Consequences

Debugging interfaces are reviewed alongside core subsystem APIs.

A change that creates important internal state with no way to inspect or explain it is considered incomplete.

Debug tooling must not silently become a mandatory on-disk feature.