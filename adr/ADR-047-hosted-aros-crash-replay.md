# ADR-047: Replay modeled power-cut images through the native AROS handler

Status: Accepted for Hosted and native-QEMU qualification

## Context

Killing Hosted MacAROS at an arbitrary instant does not produce a trustworthy
disk power-loss state. `fdsk.device` writes into a host file, and macOS/APFS may
retain writes that real hardware would lose or reorder. A process kill is still
useful for lifecycle testing, but it cannot identify an AFS+ durability point.

The portable block layer already records the exact writes and flush barriers
issued by an operation. Its power-cut model enumerates every full-write subset
of the unflushed tail plus representative torn-block writes. The intent-log
tests prove that all modeled states recover to an allowed old or new value.

## Decision

`afsplus-crash-fixtures` records an fsynced `HEAD.lock` to `HEAD` publication,
classifies every modeled crash state with the core and strict checker, and emits
six deterministic 64 MiB representative images:

1. before any new write;
2. new data written before its log record;
3. a complete log record over missing data;
4. a torn log record;
5. complete data and record immediately before the barrier; and
6. the same record after the completed barrier.

`tools/check-hosted-aros-crash-replay.sh` installs a freshly qualified handler
and target probe, boots Hosted MacAROS separately for every image, mounts it
through `fdsk.device`, and requires exactly the manifest's `old` or `new`
content. `HEAD.lock` must never survive. The host checker runs before and after
each target mount; after recovery the pending-record count must be zero.

Checker JSON schema 4 introduced `log_records_pending` (retained by schema 5),
so the gate tests replay rather than merely accepting a structurally clean
volume.

## Evidence and limits

The gate passed on 2026-08-30. The four old-state fixtures remained at
generation 2. Both valid-record fixtures replayed to generation 3. All six
post-mount images were checker-clean with zero pending records.

The gate was requalified with identification-v3 case-insensitive AROS images
after ADR-052. The same four old/two new classification held and all six final
schema-5 checker reports remained clean with zero pending records.

ADR-054 carried the identical fixtures through native Apple-AArch64 QEMU. The
four incomplete records again exposed `old` at generation 2, both valid records
exposed `new` at generation 3, and file-backed RAM extraction proved all six
final payloads checker-clean with zero pending records.

This is native-handler replay of authoritative crash artifacts, not a claim
that Hosted process termination emulates physical power loss. The exhaustive
state space remains in the portable tests; the native gate deliberately mounts
a bounded representative set. Lifecycle kills and later device failpoints are
separate tests. The same fixture contract moves next through m68k emulation and
the physical Amiga 500 in that order.

## Consequences

- Replay bugs in the AROS bridge or mount path are covered without relying on
  host-cache accidents.
- The fixture manifest is reproducible and small because images are sparse.
- The platform plan remains three targets in four validation stages: Hosted
  MacAROS, native MacAROS, then the A500 first emulated and finally physical.
