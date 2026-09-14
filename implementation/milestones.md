# Milestones and Acceptance Gates

This table is the only place that records where the project stands. Every
other document links here instead of restating status. Each status cell is one
line — `state — what passes; what is open` — and points to the design
documents that define the milestone and the test plans that qualify it. The
stage order is [ROADMAP.md](../ROADMAP.md); how a milestone is qualified is in
[testing/README.md](../testing/README.md); the history of how each state was
reached is in [NOTES.md](../NOTES.md).

Mountable Alpha-0 (M08) requires that one image support
create/read/write/truncate/rename/fsync, crash replay and a clean checker
through a host mount and a MacAROS handler. Format epoch 1 is unfrozen until
M14.

| ID | Milestone | Status | Exit criteria | Design | Test plan |
|---|---|---|---|---|---|
| M00 | Reader format frozen | Not started — identification, checkpoint and typed-node codecs are executable; every field marked TBD, Proposed or experimental is unfrozen | Reference image can be independently decoded | [03](../docs/03-on-disk-format.md), [disk-layout](../spec/disk-layout.md) | [conformance](../testing/conformance.md) |
| M01 | Portable reader | Partial — C99 seven-seed sanitizer corpus and five Rust codec fuzz targets pass; Unicode keys and remaining wire surfaces open | macOS/Linux/AROS builds, fuzz clean | [17](../docs/17-portability.md), [16](../docs/16-classic-systems.md) | [fuzzing](../testing/fuzzing.md), [conformance](../testing/conformance.md) |
| M02 | Formatter | Complete — official staged `mkafsplus`, bounded `afsplus-info`, exhaustive `afsplus-dump` and deterministic JSON/black-box gates pass | reader/formatter round-trip | [03](../docs/03-on-disk-format.md), [tools-spec](../tools/tools-spec.md) | [conformance](../testing/conformance.md) |
| M03 | RW core | Prototype complete — mutation/reflink/data-policy, bounded open-unlinked and low-space emergency-progress gates pass; symlink encoding proposed in ADR-068 | create/read/write/rename/unlink on images | [04](../docs/04-object-model.md), [05](../docs/05-directories-and-names.md), [06](../docs/06-files-and-extents.md), [07](../docs/07-allocation.md), [32](../docs/32-reflink-clone-semantics.md) | [crash-testing](../testing/crash-testing.md), [allocation](../testing/allocation-qualification.md), [shared-extents](../testing/shared-extents-qualification.md), [data-policy](../testing/data-policy-qualification.md), [orphans](../testing/orphan-qualification.md), [symlinks](../testing/symlink-qualification.md), [book review](../testing/book-review-qualification.md) |
| M04 | Journal | Prototype complete, wire experimental — Rust recovery/adapters and C namespace/truncate/one-block-COW gates pass; hardware flushes/freeze open | exhaustive crash-point suite passes | [08](../docs/08-transactions-and-journal.md) | [crash-testing](../testing/crash-testing.md), [intent-log](../testing/intent-log-write-truncate-qualification.md) |
| M05 | Checker | Prototype: checker corpus and read-only extraction pass; broader salvage and transactional repair open under Q11 | corruption corpus detected safely | [19](../docs/19-recovery-and-maintenance.md), [21](../docs/21-security-and-corruption.md) | [conformance](../testing/conformance.md), [corruption-corpus](../testing/corruption-corpus.md), [extraction](../testing/extraction-qualification.md), [developer-harness](../testing/developer-harness.md) |
| M06 | AROS handler | Partial — S0/replay on Hosted, native QEMU and both m68k emulator profiles, S1 on Hosted; Apple hardware, performance budget, physical A500 open | classic apps operate without recompilation | [aros-native-bridge](../docs/aros-native-bridge.md), [14](../docs/14-paths-and-namespaces.md) | [aros-system-volume-qualification](../testing/aros-system-volume-qualification.md) |
| M07 | FS API v2 | Portable subset — handles, cookies, logged fsync and reflinks pass; C empty create/data-free truncate/one-block COW/delete/both renames pass; broader suite open | 64-bit and capability tests pass | [13](../docs/13-filesystem-api-v2.md) | [test-strategy](../testing/test-strategy.md) |
| M08 | FUSE | Complete — real macFUSE FSKit same-image durability and AROS matrices pass; FSKit uses scoped durable data replies | same image read/write on host and AROS | [17](../docs/17-portability.md), [macos-fskit-activation](../docs/macos-fskit-activation.md) | [aros-system-volume-qualification](../testing/aros-system-volume-qualification.md) |
| M09 | Catalog | Not started | complete generation-bound backfill and multi-million object enumeration fast path | [10](../docs/10-global-catalog.md) | [performance-benchmarks](../testing/performance-benchmarks.md) |
| M10 | Change stream | Not started | gap-free enumeration/cursor handoff, reset invalidation and rescan fallback | [11](../docs/11-change-stream.md) | [security-scanning-benchmarks](../testing/security-scanning-benchmarks.md) |
| M11 | Grow resize | Not started | online/offline policy documented and tested | [19](../docs/19-recovery-and-maintenance.md) | none |
| M12 | Classic reader | Partial — heap-free C read plus cached/8 KiB namespace/truncate and allocation-validated one-block COW compile for m68k; multi-block writes/checkpoint open | constrained profile implementation demonstrated | [16](../docs/16-classic-systems.md) | [conformance](../testing/conformance.md) |
| M13 | App qualification | Partial harness — Git ref-update, checkout and log-append fsync workloads measured; Cargo/Git/Zed/Ferail/Moonstone runs open | Cargo/Git/Zed/Ferail/Moonstone workloads | [15](../docs/15-rust-zed-modern-apps.md), [31](../docs/31-extreme-workloads.md) | [application-qualification](../testing/application-qualification.md), [extreme-workload-benchmarks](../testing/extreme-workload-benchmarks.md), [benchmark-contract](../testing/benchmark-contract.md), [backup archive](../testing/backup-archive-qualification.md) |
| M14 | Epoch 1 | Preparation: directory/file/alias groups pass; whole-job archive, host security and [freeze gates](../ROADMAP.md#epoch-1-freeze-gates) open | format stability and external review | [compatibility-rules](../spec/compatibility-rules.md) | [conformance](../testing/conformance.md), [allocation](../testing/allocation-qualification.md), [security-model-conformance](../testing/security-model-conformance.md), [benchmark-contract](../testing/benchmark-contract.md) |

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
