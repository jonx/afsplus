# Tool Contract v1: volume self-description for partition tools

> **ADRs:** [ADR-025](../adr/ADR-025-structured-management-api.md),
> [ADR-037](../adr/ADR-037-intent-log.md),
> [ADR-038](../adr/ADR-038-mount-policy-and-feature-summary.md) · **Spec:** none ·
> **Tests:** none · **Milestones:** none

Status: proposal — for team review. Nothing is implemented; the vocabulary
becomes a versioned commitment once published, so review comes first. On
acceptance the spec lands under `spec/` with a numbered ADR. Decisions
requested from the team are marked **D1–D7**.

<!-- toc -->

- [Summary](#summary)
- [1. Goal and positioning](#1-goal-and-positioning)
- [2. Deliverables](#2-deliverables)
- [3. The v1 vocabulary — `ToolInfo`](#3-the-v1-vocabulary--toolinfo)
- [4. v1 capabilities — values and rationale](#4-v1-capabilities--values-and-rationale)
- [5. Design constraints](#5-design-constraints)
- [6. Implementation plan](#6-implementation-plan)
- [7. Decisions requested from the team](#7-decisions-requested-from-the-team)
- [8. Out of scope for v1](#8-out-of-scope-for-v1)
- [9. Accepted risks and limits](#9-accepted-risks-and-limits)
- [10. Estimate](#10-estimate)

<!-- /toc -->

## Summary

Give the tools that manipulate partitions — editors (GParted, OS disk
managers), imaging tools (partclone-style), installers, probes (libblkid) —
a standardized, versioned, filesystem-neutral way to ask an AFS+ volume:
identity, state, space, permitted operations, and the in-use block map. The
contract is written as a publishable neutral mini-spec that another
filesystem could adopt, but its justification is internal: a new filesystem
never gets third-party plugins written for it — it has to bring its own
answers.

Key commitments:

- **0** on-disk format changes
- **≤ 8** reads per probe, regardless of volume size
- **0** writes — the probe never mounts, therefore never replays the log
- **1** new binary: `afsplus-info`

## 1. Goal and positioning

The contract answers the five questions every partition tool asks, on every
OS:

- **Q1 — Who are you and how big do you believe you are?** Identity plus
  the geometry as seen by the filesystem; the tool compares against the
  partition size and detects disagreement.
- **Q2 — What state are you in?** Clean / intent-log records pending /
  corruption hint. New with ADR-037: a block-level clone that ignores
  pending log records silently loses fsynced operations.
- **Q3 — Space?** Used/free (O(1) from the checkpoint) and minimum shrink
  size.
- **Q4 — What am I allowed to do?** Per-operation capability bits —
  including format facts a tool cannot guess (whole-partition move is safe:
  all placement is relative to the volume start).
- **Q5 — Which blocks are actually in use?** The in-use map, served from
  the region bitmaps — the feature partclone maintains a per-filesystem
  plugin for.

**Explicit anti-goal:** no "self-describing format" that a tool would
interpret to parse AFS+ itself (Reiser4-class complexity plus attack
surface). The contract is a *closed, versioned vocabulary of answers*
served by our portable library; the tool never interprets the format.
Adoption by other filesystems is a free by-product, not an objective: the
spec is neutral and publishable, nothing more.

## 2. Deliverables

| Deliverable | Location | Content |
|---|---|---|
| `spec/tool-contract.md` | new | The neutral mini-spec: the five question groups, versioned JSON schema, C ABI sketch, conformance rules, license. |
| tool-contract ADR (number at integration) | new | Short decision record: public contract → ADR, points at the spec. |
| `afsplus-core::probe` | new module | `probe_device()` → `ToolInfo` (bounded reads, §5) and `in_use_map()` (iterator of allocated runs). |
| `afsplus-info` | binary (bin target in afsplus-check — D5) | Text + versioned `--json` + `--in-use-map`. Zero writes, stable exit codes. |
| `docs/18`, [`tools/tools-spec.md`](../tools/tools-spec.md) | update | Rewritten around the contract; afsplus-info section completed. |
| [`tools/third-party-probe-example.c`](../tools/third-party-probe-example.c) | update | Aligned with the real identification-block layout (offsets marked experimental). |

## 3. The v1 vocabulary — `ToolInfo`

| Group | Fields | Source | I/O cost |
|---|---|---|---|
| identity | `fs_name`, `format_epoch`, `uuid`, `label`, `contract_version` | identification block (LBA 0) | 1 read |
| geometry | `block_size`, `total_blocks`, `volume_bytes` | same | — |
| state | `committed_generation`, `checkpoint_status` (valid / ambiguous / none), `pending_log_records`, `needs_replay` | 2 checkpoint slots + log record headers (structural validation only — the probe is not the checker) | 2 + ≤ log_slots header reads (stop at first invalid) |
| space | `free_blocks`, `used_blocks`, `minimum_size` (D2) | checkpoint `free_blocks_total` | O(1) |
| capabilities | table in §4 | format facts + library version | — |
| features | compat / ro_compat / incompat masks (D3) | reserved — the feature framework is not executable yet | — |
| in_use_map | sorted `(start, blocks)` runs | descriptors + bitmap pages | O(bitmaps) — the one non-constant query, opt-in |

JSON output carries its own `schema_version` (same rule as ADR-025: human
output may change freely, structured output only through versioning).
Graceful degradation: if only the identification block is valid, the probe
returns the `identity` group plus a status — never a bare error. Tools need
partial answers on damaged volumes.

## 4. v1 capabilities — values and rationale

Each operation gets `supported` / `unsupported` / `conditional` (with the
condition named). v1 states what *this library* can do — not what the
format would theoretically allow (D1).

| Operation | v1 | Rationale |
|---|---|---|
| Move whole partition | supported | Format fact: all placement is relative to the volume start. No library needed — the most valuable and least guessable answer. |
| Block-level copy | conditional | Published condition: include the log area (records bind the UUID — replay works on the copy) or copy a volume with no pending records. Smart copy via `in_use_map` plus reserved areas. |
| Grow | unsupported | afsplus-resize does not exist yet. We state implementation truth, not intent. |
| Shrink | unsupported | Requires block relocation (post-Scale-1, per tools-spec). |
| Set label | unsupported | The identification block is immutable by design in the prototype (no torn write on LBA 0, ever). Making it mutable is a separate decision. |
| Set UUID | unsupported | Would invalidate the binding of intent-log records and checkpoints. |
| Check (fsck) | supported | `afsplus-check`, JSON output already exists. |

## 5. Design constraints

- **The probe never mounts.** Our `mount()` replays the log and publishes a
  checkpoint — an inspection tool must never write. The probe reads
  structures directly under strict `NO_CHANGES` semantics; invariant test:
  zero writes through the tracing backend.
- **Bounded reads.** Identification + 2 slots + log headers (stop at first
  invalid): ≤ 8 reads on the 1 TiB qualification image, asserted.
- **Nothing unchecksummed is trusted.** Every block read passes the
  existing CRC validation; a failure degrades the answer, never corrupts
  it.
- **The in-use map is exact or absent.** Served from committed state;
  quarantined blocks and reserved areas (slots, pool, log) are reported as
  in use — a cloner that follows the map reproduces a volume whose
  exhaustive check is clean, which is the qualification test.

## 6. Implementation plan

Each step has an exit gate; the order follows real dependencies.

| # | Step | Exit gate |
|---|---|---|
| 1 | Spec `tool-contract.md` + ADR-038 (vocabulary frozen after D1–D7 feedback) | Team review of this document |
| 2 | `probe::probe_device()` + `ToolInfo` | Probes of healthy / dirty / log-pending / corrupt volumes; zero writes; ≤ 8 reads on 1 TiB |
| 3 | `afsplus-info` (text, `--json`) | JSON schema stability test; exit codes |
| 4 | `in_use_map()` + `--in-use-map` | "Clone by map" qualification: copy only the listed runs plus reserved areas → remount OK + exhaustive checker clean; same with pending log (replay on the copy) |
| 5 | docs/18, tools-spec, aligned C example | Doc ↔ real layout consistency |

## 7. Decisions requested from the team

- **D1 — Capability semantics.** One axis (*what this library implements*)
  or two (*format-allows* × *implemented-here*)?
  *Recommendation: one axis in v1 — that is the tool's question; the double
  axis complicates the vocabulary for speculative benefit.*
- **D2 — `minimum_size` in v1.** `null` (honest: shrink unimplemented) or
  a theoretical bound (highest used LBA + 1, via the map — thus costly)?
  *Recommendation: `null`, with the theoretical bound as an option under
  `--in-use-map`, which computes it for free.*
- **D3 — Feature masks.** Expose empty arrays now (fields reserved in the
  schema) or wait for the feature framework to be executable?
  *Recommendation: reserve the fields now (empty + note) to avoid
  versioning the schema twice.*
- **D4 — Name and license of the publishable spec.** "AFS+ Tool Contract"
  or a neutral name (the standard ambition suggests neutrality)? License
  of the spec and the C example (workspace is BSD-2)?
  *Recommendation: neutral name in the spec ("Filesystem Tool Contract"),
  AFS+ as first implementation; BSD-2 throughout.*
- **D5 — Binary location.** Second bin target in `afsplus-check` (shares
  the lib, zero new crates) or a dedicated `afsplus-info` crate (canonical
  component list)?
  *Recommendation: bin target in afsplus-check for v1; trivial extraction
  later if the canonical list requires it.*
- **D6 — State detail level.** Expose the committed generation and record
  count (useful for debugging, reveals volume activity) or a bare
  `clean | needs_replay`?
  *Recommendation: both levels — the summary for tools, the detail under a
  clearly non-contractual `diagnostics` group.*
- **D7 — Experimental GPT GUID.** docs/18 promises a partition type GUID.
  Generate and document the experimental GUID (marked as such) in the spec
  now?
  *Recommendation: yes — zero cost, and the C example becomes complete
  end to end.*

## 8. Out of scope for v1

- C ABI exposure in `afsplus-aros-ffi` (`afsp_probe`) — natural follow-up
  once the vocabulary stabilizes.
- Upstream libblkid patch — only relevant after offsets freeze (epoch 1).
- Actual grow/shrink/relabel implementations — the contract *describes*
  them, it does not create them.

## 9. Accepted risks and limits

- **Windows/macOS will not call our library spontaneously.** The value
  there flows through our own integrations (FSKit, FUSE-T, installers) —
  the contract gives them a single source of truth, not magic ecosystem
  adoption.
- **The vocabulary is a commitment.** Once published as v1, a field changes
  only through versioning — hence review *before* implementation.
- **Offsets stay experimental until epoch 1.** The spec says so in plain
  words; the C example carries the same warning it does today.

## 10. Estimate

One working session, comparable to previous work packages: ~600–800 lines
(probe module + binary + spec), about ten new tests including two
qualifications (bounded probe on 1 TiB, clone-by-map verified by the
checker). No on-disk change, therefore no format ADR and no new crash
matrix — the invariants under test are "zero writes" and "exact answers".
