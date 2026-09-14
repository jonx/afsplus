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
| \[M01\] | Portable reader | Partial — C99 seven-seed sanitizer corpus and five Rust codec fuzz targets pass; Unicode keys and remaining wire surfaces open | macOS/Linux/AROS builds, fuzz clean | [17](../docs/17-portability.md), [16](../docs/16-classic-systems.md) | [fuzzing](../testing/fuzzing.md), [conformance](../testing/conformance.md) | [\[Stage A\]](../ROADMAP.md#stage-a-make-the-core-executable), [\[Stage D\]](../ROADMAP.md#stage-d-portability-and-host-tooling) |
| ~~M02~~ | Formatter | Complete — official staged `mkafsplus`, bounded `afsplus-info`, exhaustive `afsplus-dump` and deterministic JSON/black-box gates pass | reader/formatter round-trip | [03](../docs/03-on-disk-format.md), [tools-spec](../tools/tools-spec.md) | [conformance](../testing/conformance.md) | [\[Stage A\]](../ROADMAP.md#stage-a-make-the-core-executable) |
| \[M03\] | RW core | Prototype — mutation/reflink/pressure and core symlink crash gates pass; Rust/C codecs cross-validated; symlink replacement/adapters open | create/read/write/rename/unlink on images | [04](../docs/04-object-model.md), [05](../docs/05-directories-and-names.md), [06](../docs/06-files-and-extents.md), [07](../docs/07-allocation.md), [32](../docs/32-reflink-clone-semantics.md) | [crash-testing](../testing/crash-testing.md), [allocation](../testing/allocation-qualification.md), [shared-extents](../testing/shared-extents-qualification.md), [data-policy](../testing/data-policy-qualification.md), [orphans](../testing/orphan-qualification.md), [symlinks](../testing/symlink-qualification.md), [book review](../testing/book-review-qualification.md) | [\[Stage A\]](../ROADMAP.md#stage-a-make-the-core-executable), [\[Stage B\]](../ROADMAP.md#stage-b-resolve-the-epoch-1-architecture-blockers) |
| \[M04\] | Journal | Prototype complete, wire experimental — Rust recovery/adapters and C namespace/truncate/one-block-COW gates pass; hardware flushes/freeze open | exhaustive crash-point suite passes | [08](../docs/08-transactions-and-journal.md) | [crash-testing](../testing/crash-testing.md), [intent-log](../testing/intent-log-write-truncate-qualification.md) | [\[Stage A\]](../ROADMAP.md#stage-a-make-the-core-executable), [\[Stage B\]](../ROADMAP.md#stage-b-resolve-the-epoch-1-architecture-blockers) |
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

## Stage A executable-core audit

| Requirement | Evidence and scope | Disposition |
|---|---|---|
| Workspace, format, block, core and checker | [Block module surface](../crates/afsplus-block/src/lib.rs), formatter M02, image-operation M03 and checker M05 gates | Implemented components; preserve the independent semantic oracles. |
| Memory and sparse host-file devices | [Memory backend](../crates/afsplus-block/src/memory.rs) and [file backend](../crates/afsplus-block/src/file.rs) | Implemented host test inputs; physical-device qualification is separate. |
| Tracing, deterministic faults and simulated cuts | [Trace](../crates/afsplus-block/src/trace.rs), [faults](../crates/afsplus-block/src/fault.rs), [power cuts](../crates/afsplus-block/src/powercut.rs) and [crash matrices](../crates/afsplus-check/tests/crash_matrix.rs) | Implemented bounded-operation simulation; exhaustive coverage must be established for each publication path. |
| I/O and write-amplification accounting | Trace records successful block reads, writes, bytes and flushes | Implemented counters, [per-command CPU/RSS collection](../testing/benchmark-contract.md#per-command-host-accounting) and [phased requested-heap/I/O workload](../testing/benchmark-contract.md#phased-requested-heap-workload); [per-profile batch heap/I/O and CPU/RSS measurement](../testing/benchmark-contract.md#tree-cache-batch-measurements) passes; broader workloads, individual cache ownership and steady RSS remain open. |
| SliceBackend | [Bounded slice](../crates/afsplus-block/src/slice.rs) and [sliced-volume test](../crates/afsplus-check/tests/sliced_volume.rs) | Implemented: block isolation/error tests, format/mutate/remount/check cycle and full workspace validation pass. |
| OverlayBackend | [Bounded branches](../crates/afsplus-block/src/overlay.rs), [oracle comparison](../crates/afsplus-block/tests/overlay_replay.rs) and [filesystem branch/cut tests](../crates/afsplus-check/tests/overlay_volume.rs) | Implemented: isolation, admission, sparse-fork measurement, replay and full workspace gates pass. |
| Structured flight recorder and operation replay | [Activity events](../crates/afsplus-block/src/activity.rs) and power-cut recording provide component evidence | Deterministic [crash images](../crates/afsplus-check/src/bin/afsplus-crash-fixtures.rs), bounded [semantic runner tests](../crates/afsplus-check/tests/scenario.rs) and [semantic bundle replay/minimization](../testing/developer-harness.md#integrated-semantic-bundles-and-minimization) plus [selected-crash bundles](../testing/developer-harness.md#selected-crash-bundles) and [checker-bound verdicts](../testing/developer-harness.md#checker-bound-replay-verdicts) provide host evidence; [rebuilt-runner comparisons](../testing/developer-harness.md#comparing-a-rebuilt-runner) pass for all four cache profiles with exact artifact equality and explicit binary-identity separation. [Source-package restoration](../testing/developer-harness.md#preserving-and-restoring-working-sources), dirty/conflicted fixtures and an offline rebuilt four-profile comparison pass on the host. [Retained registry dependencies](../testing/developer-harness.md#retaining-registry-dependencies-for-a-cold-build) pass a frozen offline build with an initially empty Cargo cache, empty-source negative control and four-profile artifact comparisons. [Copied toolchain/SDK host qualification](../testing/developer-harness.md#copied-host-toolchain-and-sdk-qualification) passes with selected-linker/empty-SDK negative controls and four-profile equality. [Automated sealing/verification and reconstruction](../testing/developer-harness.md#automated-host-reconstruction) pass with real copied tools, paths containing spaces, three independent controls and four-profile artifact equality. [Common checkpoint-tail diagnostics](../testing/developer-harness.md#common-checkpoint-tail-flight-recorder) provide a bounded optional ring with retry identities and publication/adoption failure distinctions. Publication-family coverage, API-wide identities, other internal subsystems and recorder export remain open; additional host profiles belong to the Stage D portability gates. |
| Tiny-cache matrix and fuzz/property tests | [Integrated profile gates](../testing/developer-harness.md#integrated-tree-cache-profiles), [tree spill/reload tests](../crates/afsplus-core/src/cow_tree.rs) and [batch resource measurements](../testing/benchmark-contract.md#tree-cache-batch-measurements) | Partial: 2/4/8/unlimited batch, replay, snapshot/shared-survivor and allocation-cache rotation gates pass; two-page split crash matrix and early-spill retries pass. [Cache-bound ladder/replay/minimization](../testing/developer-harness.md#cache-bound-semantic-bundles) passes for all four profiles; wider mutation-family fault/artifact and fuzz/property coverage remain open. Bulk-builder and whole-heap limits, wider workload families and native resource qualification feed M12/M13 and the complete audit queue. |

Stage A remains partial because these finite requirements need implementation or
stronger evidence. Resolve bounded device wrappers before branch-based replay,
then qualify artifact reproduction and total resource accounting. Keep the full
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
| roadmap-29 | benchmark harness with CPU/RAM/I/O/flush/write-amplification accounting | [Stage A: make the core executable](../ROADMAP.md#stage-a-make-the-core-executable) | Partial | [evidence](../testing/benchmark-contract.md) |
| roadmap-30 | structured flight recorder | [Stage A: make the core executable](../ROADMAP.md#stage-a-make-the-core-executable) | Partial | [evidence](../testing/developer-harness.md) |
| roadmap-31 | tiny-cache test matrix | [Stage A: make the core executable](../ROADMAP.md#stage-a-make-the-core-executable) | Partial | [integrated profiles](../testing/developer-harness.md#integrated-tree-cache-profiles) |
| roadmap-32 | fuzzing/property tests | [Stage A: make the core executable](../ROADMAP.md#stage-a-make-the-core-executable) | Partial | [evidence](../testing/fuzzing.md) |
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
