# Milestones and Acceptance Gates

Progress notation: plain `M01` means not started, `[M01]` means started or
partial, and ~~M01~~ means complete. Stage labels follow the same convention.
Links retain the visible brackets or strikethrough. `make toc` refreshes these
labels from the milestone status cells; `make check-docs` detects stale labels.
Stage completion requires every finite contributing milestone to be complete.
An `Ongoing` status stays visible and is excluded from that calculation. Stage 0
tracks ongoing design review separately, with a plain label and explicit ongoing
scope in the stage map. An all-ongoing group cannot be marked complete. Prototype completion
with open qualification is partial.


This table is the only place that records where the project stands. Every
other document links here instead of restating status. Each status cell is one
line — `state — what passes; what is open` — and points to the design
documents that define the milestone and the test plans that qualify it. The
stage order is [ROADMAP.md](../ROADMAP.md); how a milestone is qualified is in
[testing/README.md](../testing/README.md); the history of how each state was
reached is in [NOTES.md](../NOTES.md).

The Roadmap stages column maps each milestone to the stages it contributes to.
A completed milestone does not alone complete a stage. The
[implementation phases](implementation-plan.md) link deliverables to this table.

Mountable Alpha-0 (M08) requires that one image support
create/read/write/truncate/rename/fsync, crash replay and a clean checker
through a host mount and a MacAROS handler. Format epoch 1 is unfrozen until
M14.

| ID | Milestone | Status | Exit criteria | Design | Test plan | Roadmap stages |
|---|---|---|---|---|---|---|
| M00 | Reader format frozen | Not started — identification, checkpoint and typed-node codecs are executable; every field marked TBD, Proposed or experimental is unfrozen | Epoch-1 reader contract frozen, with independent reference-image decoding and experimental-field decisions resolved | [03](../docs/03-on-disk-format.md), [disk-layout](../spec/disk-layout.md) | [conformance](../testing/conformance.md) | [\[Stage F\]](../ROADMAP.md#stage-f-production-qualification) |
| \[M01\] | Portable reader | Partial — C99 seven-seed sanitizer corpus and 18 Rust codec fuzz targets pass; Unicode keys and remaining wire surfaces open | macOS/Linux/AROS builds, fuzz clean | [17](../docs/17-portability.md), [16](../docs/16-classic-systems.md) | [fuzzing](../testing/fuzzing.md), [conformance](../testing/conformance.md) | [\[Stage A\]](../ROADMAP.md#stage-a-make-the-core-executable), [\[Stage D\]](../ROADMAP.md#stage-d-portability-and-host-tooling) |
| ~~M02~~ | Formatter | Complete — official staged `mkafsplus`, bounded `afsplus-info`, exhaustive `afsplus-dump` and deterministic JSON/black-box gates pass | reader/formatter round-trip | [03](../docs/03-on-disk-format.md), [tools-spec](../tools/tools-spec.md) | [conformance](../testing/conformance.md) | [\[Stage A\]](../ROADMAP.md#stage-a-make-the-core-executable) |
| \[M03\] | RW core | Prototype — mutation/reflink/pressure and core symlink crash gates pass; Rust/C codecs cross-validated; symlink replacement/adapters open | create/read/write/rename/unlink on images | [04](../docs/04-object-model.md), [05](../docs/05-directories-and-names.md), [06](../docs/06-files-and-extents.md), [07](../docs/07-allocation.md), [32](../docs/32-reflink-clone-semantics.md) | [crash-testing](../testing/crash-testing.md), [allocation](../testing/allocation-qualification.md), [shared-extents](../testing/shared-extents-qualification.md), [data-policy](../testing/data-policy-qualification.md), [orphans](../testing/orphan-qualification.md), [symlinks](../testing/symlink-qualification.md), [book review](../testing/book-review-qualification.md) | [\[Stage A\]](../ROADMAP.md#stage-a-make-the-core-executable), [\[Stage B\]](../ROADMAP.md#stage-b-resolve-the-epoch-1-architecture-blockers) |
| \[M04\] | Journal | Prototype complete, wire experimental — Rust recovery/preflight and C namespace/truncate/one-block-COW gates pass; hardware/freeze open | exhaustive crash-point suite passes | [08](../docs/08-transactions-and-journal.md) | [crash-testing](../testing/crash-testing.md), [intent-log](../testing/intent-log-write-truncate-qualification.md) | [\[Stage A\]](../ROADMAP.md#stage-a-make-the-core-executable), [\[Stage B\]](../ROADMAP.md#stage-b-resolve-the-epoch-1-architecture-blockers) |
| \[M05\] | Checker | Prototype: checker corpus and read-only extraction pass; broader salvage and transactional repair open under Q11 | corruption corpus detected safely | [19](../docs/19-recovery-and-maintenance.md), [21](../docs/21-security-and-corruption.md) | [conformance](../testing/conformance.md), [corruption-corpus](../testing/corruption-corpus.md), [extraction](../testing/extraction-qualification.md), [developer-harness](../testing/developer-harness.md) | [\[Stage A\]](../ROADMAP.md#stage-a-make-the-core-executable), [\[Stage F\]](../ROADMAP.md#stage-f-production-qualification) |
| \[M06\] | AROS handler | Partial — S0/replay on Hosted, native QEMU and both m68k emulator profiles, S1 on Hosted; Apple hardware, performance budget, physical A500 open | classic apps operate without recompilation | [aros-native-bridge](../docs/aros-native-bridge.md), [14](../docs/14-paths-and-namespaces.md) | [aros-system-volume-qualification](../testing/aros-system-volume-qualification.md) | [\[Stage C\]](../ROADMAP.md#stage-c-integrate-aros-and-begin-independent-c-portability) |
| \[M07\] | FS API v2 | Portable subset — handles/fsync/reflinks and scoped backup/restore components; C mutation subset; full consumer/native qualification open | 64-bit and capability tests pass | [13](../docs/13-filesystem-api-v2.md) | [test-strategy](../testing/test-strategy.md), [security/restore](../testing/security-model-conformance.md), [backup archive](../testing/backup-archive-qualification.md) | [\[Stage C\]](../ROADMAP.md#stage-c-integrate-aros-and-begin-independent-c-portability) |
| ~~M08~~ | FUSE | Complete — real macFUSE FSKit same-image durability and AROS matrices pass; FSKit uses scoped durable data replies | same image read/write on host and AROS | [17](../docs/17-portability.md), [macos-fskit-activation](../docs/macos-fskit-activation.md) | [aros-system-volume-qualification](../testing/aros-system-volume-qualification.md) | [\[Stage D\]](../ROADMAP.md#stage-d-portability-and-host-tooling) |
| M09 | Catalog | Not started | complete generation-bound backfill and multi-million object enumeration fast path | [10](../docs/10-global-catalog.md) | [performance-benchmarks](../testing/performance-benchmarks.md) | [Stage E](../ROADMAP.md#stage-e-developer-contract-accelerators-and-optional-features) |
| M10 | Change stream | Not started | gap-free enumeration/cursor handoff, reset invalidation and rescan fallback | [11](../docs/11-change-stream.md) | [security-scanning-benchmarks](../testing/security-scanning-benchmarks.md) | [Stage E](../ROADMAP.md#stage-e-developer-contract-accelerators-and-optional-features) |
| M11 | Grow resize | Not started | online/offline policy documented and tested | [19](../docs/19-recovery-and-maintenance.md) | none | [\[Stage F\]](../ROADMAP.md#stage-f-production-qualification) |
| \[M12\] | Classic reader | Partial — heap-free C read plus cached/8 KiB namespace/truncate and allocation-validated one-block COW compile for m68k; multi-block writes/checkpoint open | constrained profile implementation demonstrated | [16](../docs/16-classic-systems.md) | [conformance](../testing/conformance.md) | [\[Stage C\]](../ROADMAP.md#stage-c-integrate-aros-and-begin-independent-c-portability), [\[Stage D\]](../ROADMAP.md#stage-d-portability-and-host-tooling) |
| \[M13\] | App qualification | Partial harness — Git ref-update, checkout and log-append fsync workloads measured; Cargo/Git/Zed/Ferail/Moonstone runs open | Cargo/Git/Zed/Ferail/Moonstone workloads | [15](../docs/15-rust-zed-modern-apps.md), [31](../docs/31-extreme-workloads.md) | [application-qualification](../testing/application-qualification.md), [extreme-workload-benchmarks](../testing/extreme-workload-benchmarks.md), [benchmark-contract](../testing/benchmark-contract.md), [backup archive](../testing/backup-archive-qualification.md) | [\[Stage F\]](../ROADMAP.md#stage-f-production-qualification) |
| \[M14\] | Epoch 1 | Preparation: directory/file/alias/symlink groups pass; whole-job archive, host security and [freeze gates](../ROADMAP.md#epoch-1-freeze-gates) open | format stability and external review | [compatibility-rules](../spec/compatibility-rules.md) | [conformance](../testing/conformance.md), [allocation](../testing/allocation-qualification.md), [security-model-conformance](../testing/security-model-conformance.md), [benchmark-contract](../testing/benchmark-contract.md) | [\[Stage F\]](../ROADMAP.md#stage-f-production-qualification) |

M06 and M08 qualify AFS+ as a secondary same-image volume. The system-volume
ladder (S1–S3) and the AFS/FFS comparison contract are specified in
[testing/aros-system-volume-qualification.md](../testing/aros-system-volume-qualification.md);
a post-bootstrap `SYS:` pivot and a boot-selected AFS+ volume are separate
acceptance claims.

Platform reporting uses three targets and four ordered validation stages:
Hosted MacAROS on macOS, Amiga 500/m68k under emulation, the same classic
target on a physical A500, then native MacAROS on Apple Silicon. The emulator
is the repeatable pre-hardware gate for the second platform, not a fourth
platform; native MacAROS is ordered last because its bare-metal target must
exist before that gate can run.

<!-- toc -->

- [Early milestone boundary audit](#early-milestone-boundary-audit)
- [Scoped stage acceptance](#scoped-stage-acceptance)
- [Stage A task tracking](#stage-a-task-tracking)
  - [Structured flight recorder tasks](#structured-flight-recorder-tasks)
  - [Core diagnostic path inventory](#core-diagnostic-path-inventory)
  - [Tiny-cache test matrix tasks](#tiny-cache-test-matrix-tasks)
  - [Fuzzing and property-test tasks](#fuzzing-and-property-test-tasks)
  - [Codec surface inventory](#codec-surface-inventory)
- [Stage A executable-core audit](#stage-a-executable-core-audit)
- [Individual list-item completion](#individual-list-item-completion)

<!-- /toc -->

## Early milestone boundary audit

Completion labels remain governed by the full milestone table until the evidence
and gate mapping below are reconciled. This audit separates executable capability
from integration and final qualification without dropping any acceptance gate.

| Milestone | Executable capability evidence | Additional obligation | Closure decision |
|---|---|---|---|
| [M00](#milestones-and-acceptance-gates) | Identification, checkpoint and typed-node codecs; independent C conformance linked above | Stable epoch-1 reader contract and resolution of experimental fields | Reader-format freeze remains open; independent decoding alone does not prove freeze. |
| [\[M03\]](#milestones-and-acceptance-gates) | Image mutation and [semantic crash matrices](../crates/afsplus-check/tests/crash_matrix.rs), plus linked symlink and sharing qualification | Symlink replacement, complete operation coverage and adapter integration | Audit the bounded image-operation gate separately before changing the whole-milestone label. |
| [\[M04\]](#milestones-and-acceptance-gates) | The crash matrix includes an intentionally misordered-publication negative control and exact allowed recovery states | Each publication path, portable implementation scope, format freeze and physical provider durability | Simulated crash correctness and physical durability need distinct evidence; no hardware completion claim follows from the host suite. |
| [\[M05\]](#milestones-and-acceptance-gates) | [Corruption corpus](../crates/afsplus-check/tests/corruption_corpus.rs) checks twelve deterministic cases over six wire surfaces, exact diagnostics and reproducible exported artifacts | Coverage of additional wire surfaces, broader salvage and transactional repair under Q11 | Existing detection capability is implemented; full checker/recovery coverage remains open. |

M00 contributes to Stage F's finite epoch-1 freeze. Reader implementation and
image-operation evidence feed Stage A through M01 and M03. Continued source review
and future format evolution are ongoing activities, excluded from finite stage
completion. Remaining stage mappings require their own acceptance audit.

## Scoped stage acceptance

A stage closes when all finite gates declared in its roadmap section are
complete in this table. Ongoing rows stay visible and do not enter that
calculation. The checker requires the declaration and owner table to contain
exactly the same gate IDs. Milestone labels retain their full cross-stage scope.
Stages without an audited inventory use their contributing milestone states
conservatively. Each later stage receives its own scope audit before closure.

The [Stage A acceptance contract](../testing/developer-harness.md#stage-a-finite-acceptance)
separates executable host foundations from platform and release qualification.
The detailed evidence follows in the executable-core audit.

| Stage | Gate | Requirement | Status | Acceptance / evidence |
|---|---|---|---|---|
| Stage A | a-core | Host-independent executable core and workspace | Complete | [Workspace/core evidence](#stage-a-executable-core-audit), [core boundary](../crates/afsplus-core/src/lib.rs) |
| Stage A | a-devices | Bounded host devices, faults, slices and overlays | Complete | [Device evidence](#stage-a-executable-core-audit), [bounded overlays](../testing/developer-harness.md#bounded-overlay-branches) |
| Stage A | a-checkpoint | Format, mutate, checkpoint, cut, remount and verify | Complete | [Crash matrix](../crates/afsplus-check/tests/crash_matrix.rs), [checker verdict](../testing/developer-harness.md#checker-bound-replay-verdicts) |
| Stage A | a-accounting | Finite host CPU, heap, RSS and I/O harness | Complete | [Accounting acceptance](../testing/benchmark-contract.md#stage-a-accounting-acceptance) |
| Stage A | a-replay | Retained artifacts and qualified host reconstruction | Complete | [Automated reconstruction](../testing/developer-harness.md#automated-host-reconstruction), [replay evidence](#stage-a-executable-core-audit) |
| Stage A | a-flight | Correlated core diagnostics and bounded replay export | Partial | [Core API spans](../testing/developer-harness.md#core-api-call-spans), [coverage queue](audit-work-queue.md#complete-work-queue) |
| Stage A | a-cache | Cache-profile qualification across executable mutation families | Partial | [Integrated cache profiles](../testing/developer-harness.md#integrated-tree-cache-profiles), [coverage evidence](#stage-a-executable-core-audit) |
| Stage A | a-fuzz | Executable codec and semantic-property coverage | Partial | [Fuzz/property gates](../testing/fuzzing.md), [coverage evidence](#stage-a-executable-core-audit) |

## Stage A task tracking

This is the task-level status owner for the three unfinished roadmap entries.
The order is diagnostic export and coverage, then cache/fault coverage, then
codec/property coverage. A discovered prerequisite must be entered here with
its originating requirement or failing test before becoming a separate work
unit. Necessary fixes retain their regression and their parent gate; useful
later work stays in the full audit queue without enlarging Stage A implicitly.

### Structured flight recorder tasks

Parent: `a-flight`, roadmap entry `roadmap-30`. Origin:
[finite diagnostic acceptance](../testing/developer-harness.md#stage-a-finite-acceptance)
and [subsystem trace identities](../docs/26-debug-observability.md).

| Task | State | Evidence or next observable result |
|---|---|---|
| Common commit-tail ring, category selection, bounded live delivery and legacy export | Complete | [Commit diagnostics](../testing/developer-harness.md#common-checkpoint-tail-flight-recorder), [selected replay](../testing/developer-harness.md#selected-category-and-live-delivery-bundles) |
| API call/root/parent identities and refusal/unwind outcomes | Complete | [66-entry API scope and tests](../testing/developer-harness.md#core-api-call-spans) |
| Deferred-window identities and acknowledged versus attempted intent groups | Complete | [Window observation](../testing/developer-harness.md#deferred-window-observation); commit `9ec9591` |
| Export and replay API/window context, including explicit deferred scenarios | Complete | [Version-5 contract](../testing/developer-harness.md#api-and-window-replay-bundles); 60 retained cases, 39 legacy comparisons and full workspace qualification |
| Inventory implemented publication/subsystem paths and their missing event hooks | In progress | [Source-path inventory](#core-diagnostic-path-inventory); common-tail routing and mount attachment gap inspected; family-specific test mapping and pre-tail writes require completion |
| Correlate object/block/view identities and allocator/tree/cache/reclaim paths | In progress | [Object-map observation](../testing/developer-harness.md#object-map-observation): 18 flight tests, two recorder unit tests and 99 historical six-artifact comparisons pass; measured arm64 event 104 B, recorder 176 B. Full workspace 562 passed / 0 failed / 10 ignored; retained `build/object-observation-icc3feor`; live and captured-view export qualified below; allocator/tree/cache/reclaim hooks remain open |
| Publication-family observation equivalence baseline | Complete | [Eight-family matrix](../testing/developer-harness.md#publication-family-observation-equivalence): ID-preserving oracle passes with legacy and object observation across eight families, four cache profiles, three flush outcomes and two ring capacities; full workspace 562/0/10, retained `build/object-observation-icc3feor`. Prior `build/publication-families-92wlsaqm` retains its narrower success/error scope |
| Qualify export and event loss for the added core paths | In progress | [Object wire contract](../testing/developer-harness.md#object-map-replay-bundles): 24 live-view combinations, 12 deferred cases, reduction and five cut variants retained; 99 historical six-artifact comparisons, 28 Python tests and full Rust 573/0/10 pass in `build/object-export-i1lx25ko`. [Captured replay](../testing/developer-harness.md#captured-snapshot-replay-bundles) adds 48 retained cases plus a reduced negative and a 2 MiB sparse failure; 142 historical comparisons, full Rust 576/0/10 and corrected Python 34 tests pass in `build/captured-replay-k_2r6p2h`. Other subsystem exports remain open |

Close `a-flight` only when these finite core tasks have evidence. Native adapters,
external consumer schedules and future unimplemented subsystems retain their
[full-queue owners](audit-work-queue.md#complete-work-queue).

### Core diagnostic path inventory

Owner: `a-flight` / `roadmap-30`, board task 2 (`codex`). This inventory
separates source routing from executed diagnostic qualification. A common
helper does not prove that every caller has an observation-equivalence test.

| Path | Inspected evidence | Coverage disposition and next check |
|---|---|---|
| Ordinary checkpoint publication | [Volume](../crates/afsplus-core/src/volume.rs), `commit_transaction` → `commit_transaction_inner` → `commit_transaction_body` | Common tail emits begin, data completion, metadata durability, publication, checkpoint durability and adoption/failure; enumerate caller families and their named normal/refusal/fault tests |
| Snapshot registry publication | [Snapshot operations](../crates/afsplus-core/src/volume/snapshots.rs), `commit_snapshot_change` calls `commit_transaction_inner` | Same tail; map create/delete/maintenance and retained-view tests separately rather than inferring full coverage from routing |
| Deferred-window publication | [Volume](../crates/afsplus-core/src/volume.rs), pending-window object-map publication calls `commit_transaction_inner` | Same tail and window identity; inspect writes before this call independently of `DataWritesComplete` |
| Mount selection and intent recovery | [Mount](../crates/afsplus-core/src/mount.rs), `mount_configured` constructs the volume and invokes `recover_intent_log` or `inspect_intent_log` before returning it | Missing caller-supplied recorder during mount; design bounded attachment before selection/recovery and compare successful, refused and damaged-log mounts without changing mount semantics |
| Allocator, tree, cache and reclamation | [Event vocabulary](../crates/afsplus-core/src/flight.rs), [allocation attribution](../crates/afsplus-core/src/allocation_trace.rs) | No dedicated subsystem event kinds in this vocabulary; allocation-domain attribution is resource accounting, not an object/block/view event trace. Enumerate subsystem transitions and their test owners before adding hooks |

The direct publication callers in [Volume](../crates/afsplus-core/src/volume.rs)
are `reclaim_step_untraced`, `clone_file_untraced`, `clone_range_untraced`,
`create_leaf_in_directory`, `create_directory_untraced`,
`cleanup_orphan_data_step`, `ensure_orphan_directory`, `remove_entry`,
`link_file_untraced`, `rename_internal`, `commit_staged_file_layout` and
`materialize_batch`. Metadata adds `commit_object_metadata`; snapshots add
`commit_snapshot_change`. Each caller needs family-specific evidence; the
wrapper `commit_transaction` is not an additional user-operation family.

| Additional path | Source and existing diagnostic test | Missing evidence / implementation dependency |
|---|---|---|
| Window create write-through, existing-file writes and truncate tail zeroing | [Volume](../crates/afsplus-core/src/volume.rs), `apply_batch_op`, window write/truncate implementations; [flight tests](../crates/afsplus-core/tests/flight.rs), `window_data_failures_distinguish_discarded_mutations_from_retained_fsync_state` | Four-profile mutation/flush failure comparisons exist; per-object/logical-range/physical-block events are missing before common-tail entry |
| Intent-group durability and empty-group fsync | [Volume](../crates/afsplus-core/src/volume.rs), `window_fsync_untraced`; [flight tests](../crates/afsplus-core/tests/flight.rs), `deferred_windows_join_api_calls_groups_and_commits_without_changing_io` | Group begin/durable/failure identifies log publication; the preceding existing-file data barrier and empty-group flush need distinct I/O event ownership |
| Dirty tree-cache eviction | [COW tree](../crates/afsplus-core/src/cow_tree.rs), `enforce_cache_limit` writes staged blocks before final publication | Spill counters exist; block identity, eviction/reload and write-failure events require bounded propagation into tree edits. Coordinate tests with `a-cache`, without treating cache correctness as diagnostic qualification |
| Format publication | [Formatter](../crates/afsplus-core/src/mkfs.rs), metadata/identification barrier then slot-A publication barrier | Formatter takes a device, not a Volume recorder; explicit observation entry and interrupted-format comparisons are needed. Preserve its non-atomic formatting contract |
| Standalone verification | [Verifier](../crates/afsplus-core/src/verify.rs), `load_mount_state`, `load_committed_state`, `full_sweep` | Structured findings and block I/O are distinct from flight events; define error-location correlation without making exhaustive verification part of normal mount |
| API guard registration | [API coverage test](../crates/afsplus-core/tests/api_coverage.rs), `every_mutable_operational_entry_has_a_registered_outer_guard` | Source registration proves guard presence only; it does not execute each method or prove its subsystem coverage |
| Snapshot metadata and busy-delete outcomes | [Flight tests](../crates/afsplus-core/tests/flight.rs), `api_snapshot_and_metadata_calls_preserve_captured_state_and_busy_refusals` | Four-profile protection/captured-state/busy-delete image and I/O comparisons exist; this does not cover every maintenance/reclaim failure or attach view IDs to events |

Next implementation order: establish object/block/view context and bounded
subsystem propagation; instrument pre-tail writes and allocation/tree/cache/
reclaim transitions; attach observation before mount recovery and to standalone
format/verification entry points; qualify every named publication family and
export/loss path. Existing wire versions must retain their replay contracts.

This is a partial source audit, not closure of the inventory task. The rows
identify formatting, verification and pre-tail gaps; detailed subsystem
transition coverage and test ownership for every publication caller require
completion. Platform adapters keep their
separate qualification owners in the [audit queue](audit-work-queue.md).

### Tiny-cache test matrix tasks

Parent: `a-cache`, roadmap entry `roadmap-31`. Origin:
[finite cache acceptance](../testing/developer-harness.md#stage-a-finite-acceptance).

| Task | State | Evidence or next observable result |
|---|---|---|
| Baseline 2/4/8/unlimited profiles, staged-tree spills and replay policy | Complete | [Integrated cache profiles](../testing/developer-harness.md#integrated-tree-cache-profiles), [cache-bound replay](../testing/developer-harness.md#cache-bound-semantic-bundles) |
| Map every executable mutation/publication family to cache and fault tests | In progress | [26-family source inventory](../crates/afsplus-check/tests/tiny_cache_matrix.md); explicit baseline omissions and added executable coverage; completeness review required |
| Fill uncovered family/profile combinations | In progress | [Ten-test matrix](../crates/afsplus-check/tests/tiny_cache_matrix.md): 12 namespace/metadata families at 2/4/8/unlimited, 72,220 modeled cuts; integrated target 10 passed / 0 failed / 0 ignored with be97dbe, retained `build/cache-matrix-review-0vur3dno`; Aligned CloneRange reference-boundary cuts pass at 2/4/8/unlimited with exact source/destination/peer and refcount oracles ([profile scope](../crates/afsplus-check/tests/tiny_cache_matrix.md)); First-sharing CloneFile cuts also pass in all four profiles with exact source/clone bytes and two-reference run checks; First-clone before-write/flush faults and remount retries also pass: 60 injected failures over four profiles; Create/CloneFile completed-write and adoption-read errors pass 16 profile/fault/family combinations with post-remount mutation and exact survivor checks; eviction, retained views and other shared-family faults open |
| Fill missing spill/failure/recovery combinations | In progress | [Matrix oracles and limits](../crates/afsplus-check/tests/tiny_cache_matrix.md): 576 write/barrier faults, 1,384 replay cuts, 12 early spill faults plus bounded reservation refusal/retry; remaining transitions listed in inventory |

Close `a-cache` when the inventory has no uncovered finite host combinations.
Whole-job memory budgets, aged sustained workloads and native memory pressure
belong to M12/M13 and the [cache/VM integration queue](audit-work-queue.md#complete-work-queue).

### Fuzzing and property-test tasks

Parent: `a-fuzz`, roadmap entry `roadmap-32`. Origin:
[required properties and target matrix](../testing/fuzzing.md).

| Task | State | Evidence or next observable result |
|---|---|---|
| Eighteen codec targets and deterministic mutation/replay controls | Complete | [Rust codec gate](../testing/fuzzing.md#rust-codec-gate) |
| Baseline generated semantic scenarios with exact byte/prefix oracles | Complete | [Seeded properties](../testing/fuzzing.md#seeded-semantic-properties) |
| Audit executable wire surfaces against the target matrix | In progress | [Codec surface inventory](#codec-surface-inventory) maps direct dispatch omissions and seed-shape gaps; operation generators and caller validation remain to audit |
| Add missing executable codec targets and semantic operation generators | In progress | Five direct snapshot targets qualified: 12 total targets x 4096 cases, eight fuzz tests, independent fields/length/boundary oracles; retained `build/snapshot-codec-integration-ijm6odz7`. Three reclaim targets also qualified: 15 total targets x 4096 cases, 11 fuzz tests and 11 saved replay controls; retained `build/reclaim-codec-integration-3ugcpze0`. Snapshot-bearing checkpoint target 16 also qualified: 65,536 cases, 13 fuzz tests, 12 saved replay controls; retained `build/snapshot-checkpoint-integration-wjhshbg2`. Object targets 17–18 qualified: 73,728 cases, 15 fuzz tests, 14 saved replays; retained `build/object-codec-integration-ojy8u7x0`. Other listed codecs and semantic generators remain open |
| Retain and replay failure/property cases across cache profiles | To do | Reproducible artifacts and negative controls for each newly covered family |

Close `a-fuzz` when executable host surfaces and operation families have the
required coverage. Frozen-field decisions, portable C qualification and future
catalog/change-stream implementations keep their separate stage owners.

### Codec surface inventory

Owner: `a-fuzz` / `roadmap-32`, board task 3 (`codex`). Source inspection
of [the fuzz dispatcher](../fuzz/src/lib.rs) distinguishes an executed target
from an implemented decoder and a seed from a complete semantic family.

| Surface | Inspected dispatch / seed | Required next coverage |
|---|---|---|
| Identification, checkpoint, tree node, object record, intent log, bitmap page, region descriptor | Seven direct decoder targets; deterministic mutation and accepted-input re-encoding | Preserve existing artifact target IDs and seed reproducibility when extending the campaign |
| Snapshot-bearing checkpoint | [Direct target 16](../fuzz/src/checkpoint_snapshot.rs) supplies explicit snapshot roots, independent decoded fields and fixed-geometry structural admission; resealed length/root controls | Feature negotiation and referenced-root ownership require mount and semantic-operation validation |
| Snapshot leaf values and keys | [Snapshot codecs](../crates/afsplus-format/src/snapshot.rs) decode registry control, snapshot records, lifetime records, ledger state and keys; direct targets 8–12, fixed-context independent admission and field oracles qualified | Extend context coverage and caller/tree/cross-record validation; direct leaf coverage does not qualify ownership or visibility |
| Reclaim root, segment and table | [Reclaim codecs](../crates/afsplus-format/src/reclaim.rs) have direct targets 13–15 with independent field/admission oracles, structured seeds, truncation/resealed corruption and capacity/count/cursor relations | Queue ordering, ownership and volume-bound validation belong to caller and semantic-operation qualification |
| Inline symlink and metadata-specific object decoding | [Object codecs](../crates/afsplus-format/src/object.rs) have direct targets 17–18 with explicit symlink and object-type seeds, independent fields/admission, UTF-8/length/flags/tail and borrowed-target checks, and short-buffer controls | Cross-record ownership and caller semantics require operation tests; [Q13](open-questions.md) owns generic header/extension/tail admission before format freeze |
| Legacy directory, object map and retired list | [Directory](../crates/afsplus-format/src/dir.rs), [object map](../crates/afsplus-format/src/omap.rs), [retired list](../crates/afsplus-format/src/retired.rs); [roundtrip tests](../crates/afsplus-format/tests/roundtrip.rs) exercise them; [reserved-byte regression](../crates/afsplus-format/tests/legacy_reserved.rs) rejects 45 resealed corruptions, but direct fuzz dispatch omits them | Preserve these executable readers in the audit even when mounted core paths use newer trees; qualify them or document an explicit removal decision |
| Tree payload meaning and cross-record relations | Generic tree target decodes node structure from an ObjectMap leaf seed; [directory](../crates/afsplus-core/src/directory.rs), [extent map](../crates/afsplus-core/src/extent_map.rs), [object map](../crates/afsplus-core/src/object_map.rs), [allocation root](../crates/afsplus-core/src/allocation_root.rs) and [shared extents](../crates/afsplus-core/src/shared_extents.rs) validate payloads against geometry and ownership | Add generated valid/invalid payloads through core entry points, including range overflow, refcounts and canonical ordering; generic tree admission does not prove application-level validity |
| Directory spelling and comparison keys | [Name-key validation](../crates/afsplus-core/src/name_key.rs) checks the stored key against the versioned normalization algorithm | Generated Unicode spelling/key pairs, length bounds and wrong-key rejection; preserve original spelling and locale independence |
| Intent record versus replay sequence | Seed contains create/delete/rename/write/truncate operations within one record | Separate multi-record generation/sequence/prefix and restartability properties from single-record codec admission |

This inventory does not close `a-fuzz`. Tree payload-specific validators, core
mapping codecs, operation generators and retained failing-case coverage need
an explicit disposition before the finite gate can close. Plain helper modules
such as endian and checksum routines are dependencies, not automatically
separate semantic targets.

## Stage A executable-core audit

| Requirement | Evidence and scope | Disposition |
|---|---|---|
| Workspace, format, block, core and checker | [Block module surface](../crates/afsplus-block/src/lib.rs), formatter M02, image-operation M03 and checker M05 gates | Implemented components; preserve the independent semantic oracles. |
| Memory and sparse host-file devices | [Memory backend](../crates/afsplus-block/src/memory.rs) and [file backend](../crates/afsplus-block/src/file.rs) | Implemented host test inputs; physical-device qualification is separate. |
| Tracing, deterministic faults and simulated cuts | [Trace](../crates/afsplus-block/src/trace.rs), [faults](../crates/afsplus-block/src/fault.rs), [power cuts](../crates/afsplus-block/src/powercut.rs) and [crash matrices](../crates/afsplus-check/tests/crash_matrix.rs) | Implemented bounded-operation simulation; exhaustive coverage must be established for each publication path. |
| I/O and write-amplification accounting | Trace records successful block reads, writes, bytes and flushes | Implemented counters, [per-command CPU/RSS collection](../testing/benchmark-contract.md#per-command-host-accounting) and [phased requested-heap/I/O workload](../testing/benchmark-contract.md#phased-requested-heap-workload); [per-profile batch heap/I/O and CPU/RSS measurement](../testing/benchmark-contract.md#tree-cache-batch-measurements) passes; [phase-boundary RSS and 16-round read series](../testing/benchmark-contract.md#phase-boundary-resident-memory-and-repeated-reads) pass on macOS for small files and all four cache profiles with unchanged images and explicit observer costs. [Allocation-origin accounting](../testing/benchmark-contract.md#allocation-origins-and-instrumentation-cost) balances across small files and all cache profiles with unchanged image/I/O results; header/padding costs are separate. [Borrowed batch payloads](../testing/benchmark-contract.md#borrowed-payloads-in-atomic-batches) remove per-block copies with four-profile semantic/fault/crash tests and unchanged measured images/I/O. The [finite Stage A harness gate](../testing/benchmark-contract.md#stage-a-accounting-acceptance) is complete; broader steady-state workloads, current ownership and other-platform qualification remain open under the resource/application queue. |
| SliceBackend | [Bounded slice](../crates/afsplus-block/src/slice.rs) and [sliced-volume test](../crates/afsplus-check/tests/sliced_volume.rs) | Implemented: block isolation/error tests, format/mutate/remount/check cycle and full workspace validation pass. |
| OverlayBackend | [Bounded branches](../crates/afsplus-block/src/overlay.rs), [oracle comparison](../crates/afsplus-block/tests/overlay_replay.rs) and [filesystem branch/cut tests](../crates/afsplus-check/tests/overlay_volume.rs) | Implemented: isolation, admission, sparse-fork measurement, replay and full workspace gates pass. |
| Structured flight recorder and operation replay | [Activity events](../crates/afsplus-block/src/activity.rs) and power-cut recording provide component evidence | Deterministic [crash images](../crates/afsplus-check/src/bin/afsplus-crash-fixtures.rs), bounded [semantic runner tests](../crates/afsplus-check/tests/scenario.rs) and [semantic bundle replay/minimization](../testing/developer-harness.md#integrated-semantic-bundles-and-minimization) plus [selected-crash bundles](../testing/developer-harness.md#selected-crash-bundles) and [checker-bound verdicts](../testing/developer-harness.md#checker-bound-replay-verdicts) provide host evidence; [rebuilt-runner comparisons](../testing/developer-harness.md#comparing-a-rebuilt-runner) pass for all four cache profiles with exact artifact equality and explicit binary-identity separation. [Source-package restoration](../testing/developer-harness.md#preserving-and-restoring-working-sources), dirty/conflicted fixtures and an offline rebuilt four-profile comparison pass on the host. [Retained registry dependencies](../testing/developer-harness.md#retaining-registry-dependencies-for-a-cold-build) pass a frozen offline build with an initially empty Cargo cache, empty-source negative control and four-profile artifact comparisons. [Copied toolchain/SDK host qualification](../testing/developer-harness.md#copied-host-toolchain-and-sdk-qualification) passes with selected-linker/empty-SDK negative controls and four-profile equality. [Automated sealing/verification and reconstruction](../testing/developer-harness.md#automated-host-reconstruction) pass with real copied tools, paths containing spaces, three independent controls and four-profile artifact equality. [Common checkpoint-tail diagnostics](../testing/developer-harness.md#common-checkpoint-tail-flight-recorder) provide a bounded optional ring with retry identities and publication/adoption failure distinctions. [Internal diagnostic bundles](../testing/developer-harness.md#internal-diagnostic-bundles) bind commit events and loss to semantic operations, replay and minimization at all four cache profiles. [Category filtering and bounded live delivery](../testing/developer-harness.md#category-selection-and-live-diagnostics) pass four-profile image/I/O/failure comparisons, including saturation and disconnection. [Version-4 category/live bundles](../testing/developer-harness.md#selected-category-and-live-delivery-bundles) pass 640 in-process combinations and fresh-process replay/admission/minimization controls. [Core API spans](../testing/developer-harness.md#core-api-call-spans) cover 66 mutable entries with nested root/span identities, commit correlation, early refusal and unwind restoration; four-profile image/I/O checks include all data/metadata/checkpoint barriers and snapshot/protection operations. [Deferred-window observation](../testing/developer-harness.md#deferred-window-observation) joins staged calls, intent groups and checkpoint attempts with four-profile image/I/O and failure checks. [Version-5 replay](../testing/developer-harness.md#api-and-window-replay-bundles) covers API/window export, deferred groups and selected-cut remount oracles. Publication-family coverage, object/view linkage, platform API scope and other internal subsystems remain open; additional host profiles belong to the Stage D portability gates. |
| Tiny-cache matrix and fuzz/property tests | [Integrated profile gates](../testing/developer-harness.md#integrated-tree-cache-profiles), [tree spill/reload tests](../crates/afsplus-core/src/cow_tree.rs) and [batch resource measurements](../testing/benchmark-contract.md#tree-cache-batch-measurements) | Partial: 2/4/8/unlimited batch, replay, snapshot/shared-survivor and allocation-cache rotation gates pass; two-page split crash matrix and early-spill retries pass. [Cache-bound ladder/replay/minimization](../testing/developer-harness.md#cache-bound-semantic-bundles) passes for all four profiles. [Seeded semantic properties](../testing/fuzzing.md#seeded-semantic-properties) pass 24 three-seed/prefix/profile cases with fresh replay and a wrong-byte negative control; [Eighteen codec targets](../testing/fuzzing.md#rust-codec-gate) include allocation bitmaps and region bindings; uncovered codec surfaces and mutation-family fault/artifact coverage remain open. Bulk-builder and whole-heap limits, wider workload families and native resource qualification feed M12/M13 and the complete audit queue. |

Stage A remains partial: API-wide/subsystem diagnostics and wider
cache/fault/property coverage require their own evidence. Bounded device wrappers,
replay reconstruction and the finite host-accounting harness have qualified
implementations. Later workload and platform obligations remain in the full
[audit queue](audit-work-queue.md#complete-work-queue), including later consumers
and physical-provider gates, in the dependency order.

## Individual list-item completion

These records drive strikethrough on individual roadmap and implementation-plan
entries. A completed implementation item does not close its phase's separate
acceptance tests, platform qualification or format freeze. Unregistered entries
remain unmarked until their full stated requirement has evidence. Ongoing work
is never struck through. Stable hidden markers bind each generated list entry
to one record; edit status here and regenerate navigation. The Task and Stage /
phase columns are generated from the source list text and its containing heading,
so each identifier has a readable description and a direct navigation link.

| Item | Task | Stage / phase | Status | Evidence |
|---|---|---|---|---|
| roadmap-01 | establish Rust workspace | [Stage A: make the core executable](../ROADMAP.md#stage-a-make-the-core-executable) | Complete | [evidence](../Cargo.toml) |
| roadmap-02 | `afsplus-format` | [Stage A: make the core executable](../ROADMAP.md#stage-a-make-the-core-executable) | Complete | [evidence](../crates/afsplus-format/src/lib.rs) |
| roadmap-03 | `afsplus-block` | [Stage A: make the core executable](../ROADMAP.md#stage-a-make-the-core-executable) | Complete | [evidence](../crates/afsplus-block/src/lib.rs) |
| roadmap-04 | `afsplus-core` | [Stage A: make the core executable](../ROADMAP.md#stage-a-make-the-core-executable) | Complete | [evidence](../crates/afsplus-core/src/lib.rs) |
| roadmap-05 | `afsplus-check` | [Stage A: make the core executable](../ROADMAP.md#stage-a-make-the-core-executable) | Complete | [evidence](../crates/afsplus-check/src/lib.rs) |
| roadmap-06 | sparse raw host-file backend | [Stage A: make the core executable](../ROADMAP.md#stage-a-make-the-core-executable) | Complete | [evidence](../crates/afsplus-block/src/file.rs) |
| roadmap-07 | memory block backend | [Stage A: make the core executable](../ROADMAP.md#stage-a-make-the-core-executable) | Complete | [evidence](../crates/afsplus-block/src/memory.rs) |
| roadmap-08 | trace wrapper | [Stage A: make the core executable](../ROADMAP.md#stage-a-make-the-core-executable) | Complete | [evidence](../crates/afsplus-block/src/trace.rs) |
| roadmap-09 | deterministic fault-injection wrapper | [Stage A: make the core executable](../ROADMAP.md#stage-a-make-the-core-executable) | Complete | [evidence](../crates/afsplus-check/tests/faults.rs) |
| roadmap-10 | power-cut simulation backend | [Stage A: make the core executable](../ROADMAP.md#stage-a-make-the-core-executable) | Complete | [evidence](../crates/afsplus-check/tests/crash_matrix.rs) |
| roadmap-11 | minimal format/checkpoint descriptor | [Stage A: make the core executable](../ROADMAP.md#stage-a-make-the-core-executable) | Complete | [evidence](../crates/afsplus-format/src/checkpoint.rs) |
| roadmap-12 | checkpoint slots A/B | [Stage A: make the core executable](../ROADMAP.md#stage-a-make-the-core-executable) | Complete | [evidence](../crates/afsplus-check/tests/crash_matrix.rs) |
| roadmap-13 | root object | [Stage A: make the core executable](../ROADMAP.md#stage-a-make-the-core-executable) | Complete | [evidence](../crates/afsplus-check/tests/basic.rs) |
| roadmap-14 | minimal metadata encoding | [Stage A: make the core executable](../ROADMAP.md#stage-a-make-the-core-executable) | Complete | [evidence](../crates/afsplus-format/src/object.rs) |
| roadmap-15 | first create-object transaction | [Stage A: make the core executable](../ROADMAP.md#stage-a-make-the-core-executable) | Complete | [evidence](../crates/afsplus-check/tests/basic.rs) |
| roadmap-16 | remount/invariant checker | [Stage A: make the core executable](../ROADMAP.md#stage-a-make-the-core-executable) | Complete | [checker corpus](../crates/afsplus-check/tests/corruption_corpus.rs), [checked replay](../crates/afsplus-check/tests/scenario.rs) |
| roadmap-17 | deterministic crash matrix after every write/flush | [Stage A: make the core executable](../ROADMAP.md#stage-a-make-the-core-executable) | Complete | [evidence](../crates/afsplus-check/tests/crash_matrix.rs) |
| roadmap-18 | SliceBackend for partition/disk-image viewports | [Stage A: make the core executable](../ROADMAP.md#stage-a-make-the-core-executable) | Complete | [evidence](../crates/afsplus-check/tests/sliced_volume.rs) |
| roadmap-19 | OverlayBackend for cheap writable test branches | [Stage A: make the core executable](../ROADMAP.md#stage-a-make-the-core-executable) | Complete | [evidence](../crates/afsplus-check/tests/overlay_volume.rs) |
| roadmap-20 | operation record/replay | [Stage A: make the core executable](../ROADMAP.md#stage-a-make-the-core-executable) | Complete | [evidence](../tools/test-afsptest.py) |
| roadmap-21 | B+ tree directories | [Stage B / B4. Core filesystem structures](../ROADMAP.md#b4-core-filesystem-structures) | Complete | [evidence](../crates/afsplus-core/src/directory.rs) |
| roadmap-22 | extent mapping | [Stage B / B4. Core filesystem structures](../ROADMAP.md#b4-core-filesystem-structures) | Complete | [evidence](../crates/afsplus-check/tests/basic.rs) |
| roadmap-23 | sparse files | [Stage B / B4. Core filesystem structures](../ROADMAP.md#b4-core-filesystem-structures) | Complete | [evidence](../crates/afsplus-check/tests/data_policy.rs) |
| roadmap-24 | shared-extent/reference prototype for reflinks | [Stage B / B4. Core filesystem structures](../ROADMAP.md#b4-core-filesystem-structures) | Complete | [evidence](../crates/afsplus-check/tests/shared_extents.rs) |
| roadmap-25 | CloneFile/CloneRange semantics | [Stage B / B4. Core filesystem structures](../ROADMAP.md#b4-core-filesystem-structures) | Complete | [evidence](../crates/afsplus-check/tests/shared_clone.rs) |
| roadmap-26 | deferred reclamation | [Stage B / B4. Core filesystem structures](../ROADMAP.md#b4-core-filesystem-structures) | Complete | [evidence](../crates/afsplus-check/tests/reclaim.rs) |
| roadmap-27 | checker | [Stage B / B4. Core filesystem structures](../ROADMAP.md#b4-core-filesystem-structures) | Complete | [evidence](../crates/afsplus-check/tests/corruption_corpus.rs) |
| roadmap-28 | FUSE host mount | [Stage D: portability and host tooling](../ROADMAP.md#stage-d-portability-and-host-tooling) | Complete | [evidence](../testing/aros-system-volume-qualification.md) |
| roadmap-29 | benchmark harness with CPU/RAM/I/O/flush/write-amplification accounting | [Stage A: make the core executable](../ROADMAP.md#stage-a-make-the-core-executable) | Complete | [accounting acceptance](../testing/benchmark-contract.md#stage-a-accounting-acceptance) |
| roadmap-30 | structured flight recorder | [Stage A: make the core executable](../ROADMAP.md#stage-a-make-the-core-executable) | Partial | [evidence](../testing/developer-harness.md) |
| roadmap-31 | tiny-cache test matrix | [Stage A: make the core executable](../ROADMAP.md#stage-a-make-the-core-executable) | Partial | [integrated profiles](../testing/developer-harness.md#integrated-tree-cache-profiles) |
| roadmap-32 | fuzzing/property tests | [Stage A: make the core executable](../ROADMAP.md#stage-a-make-the-core-executable) | Partial | [evidence](../testing/fuzzing.md) |
| roadmap-33 | keep core disk semantics independent from host namespaces | [Stage A: make the core executable](../ROADMAP.md#stage-a-make-the-core-executable) | Ongoing | [namespace boundary](../adr/ADR-017-namespace-outside-format.md), [core dependencies](../crates/afsplus-core/Cargo.toml) |
| implementation-01 | host-file block backend | [Phase 1: portable reader](implementation-plan.md#phase-1-portable-reader) | Complete | [evidence](../crates/afsplus-block/src/file.rs) |
| implementation-02 | superblock discovery | [Phase 1: portable reader](implementation-plan.md#phase-1-portable-reader) | Complete | [evidence](../crates/afsplus-check/tests/mount_modes.rs) |
| implementation-03 | feature negotiation | [Phase 1: portable reader](implementation-plan.md#phase-1-portable-reader) | Complete | [evidence](../crates/afsplus-check/tests/mount_modes.rs) |
| implementation-04 | object read | [Phase 1: portable reader](implementation-plan.md#phase-1-portable-reader) | Complete | [evidence](../crates/afsplus-check/tests/basic.rs) |
| implementation-05 | directory lookup/iteration | [Phase 1: portable reader](implementation-plan.md#phase-1-portable-reader) | Complete | [evidence](../crates/afsplus-check/tests/basic.rs) |
| implementation-06 | extent read | [Phase 1: portable reader](implementation-plan.md#phase-1-portable-reader) | Complete | [evidence](../crates/afsplus-check/tests/basic.rs) |
| implementation-07 | metadata validation | [Phase 1: portable reader](implementation-plan.md#phase-1-portable-reader) | Complete | [evidence](../crates/afsplus-check/tests/corruption_corpus.rs) |
| implementation-08 | deterministic test mode | [Phase 2: formatter and image builder](implementation-plan.md#phase-2-formatter-and-image-builder) | Complete | [evidence](../testing/conformance.md) |
| implementation-09 | round-trip reader tests | [Phase 2: formatter and image builder](implementation-plan.md#phase-2-formatter-and-image-builder) | Complete | [evidence](../crates/afsplus-check/tests/basic.rs) |
| implementation-10 | no native struct serialization | [Phase 2: formatter and image builder](implementation-plan.md#phase-2-formatter-and-image-builder) | Complete | [evidence](../crates/afsplus-format/src/lib.rs) |
| implementation-11 | allocation regions | [Phase 3: allocator and mutations](implementation-plan.md#phase-3-allocator-and-mutations) | Complete | [evidence](../testing/allocation-qualification.md) |
| implementation-12 | file create/write/truncate | [Phase 3: allocator and mutations](implementation-plan.md#phase-3-allocator-and-mutations) | Complete | [evidence](../crates/afsplus-check/tests/basic.rs) |
| implementation-13 | mkdir | [Phase 3: allocator and mutations](implementation-plan.md#phase-3-allocator-and-mutations) | Complete | [evidence](../crates/afsplus-check/tests/basic.rs) |
| implementation-14 | unlink | [Phase 3: allocator and mutations](implementation-plan.md#phase-3-allocator-and-mutations) | Complete | [evidence](../crates/afsplus-check/tests/basic.rs) |
| implementation-15 | rename | [Phase 3: allocator and mutations](implementation-plan.md#phase-3-allocator-and-mutations) | Complete | [evidence](../crates/afsplus-check/tests/crash_matrix.rs) |
| implementation-16 | hard links | [Phase 3: allocator and mutations](implementation-plan.md#phase-3-allocator-and-mutations) | Complete | [hard-link lifetime test](../crates/afsplus-check/tests/basic.rs) |
| implementation-17 | symlinks | [Phase 3: allocator and mutations](implementation-plan.md#phase-3-allocator-and-mutations) | Partial | [evidence](../testing/symlink-qualification.md) |
| implementation-18 | fuzz targets active | [Phase 1: portable reader](implementation-plan.md#phase-1-portable-reader) | Complete | [active codec targets](../testing/fuzzing.md) |
| implementation-19 | FUSE interop | [Phase 13: epoch 1 freeze](implementation-plan.md#phase-13-epoch-1-freeze) | Complete | M08; [same-image FUSE/AROS qualification](../testing/aros-system-volume-qualification.md) |
