# Notes

The project journal: what was decided, tried and delivered, newest first.
This is the only document that narrates. Everything else states the finished
state and links here for the story; see `docs/DOCUMENTATION.md` for the
rules.

Entry format: `## YYYY-MM-DD — title`.

## 2026-08-31 — Documentation restructured around one home per fact

The repository's documentation is reorganised so that state, design,
procedure and history each have exactly one home
([README.md § Documentation map](README.md#documentation-map)), every document
is reachable from an index, and `tools/check-docs.py` enforces the mechanical
part of the contract (`make check-docs`).

Measured before the restructuring: 132 Markdown files (13,785 lines) outside
the code trees; 82 files carried a `Status:` line (60 ADRs and 22 others); 365
journey words ("now", "still", "remains", ...); 2 Markdown links between
documents against 118 backticked path mentions (9 of them not resolving from
the repository root); 59 documents referenced from nowhere; 31 files longer
than 150 lines, none with a table of contents; `FILE_INDEX.md` and
`SHA256SUMS.json` generated once in the initial commit and never again.

What moved where:

- `implementation/milestones.md` is the only status authority. `README.md`
  keeps a five-row summary; `ROADMAP.md` carries the stage plan only and links
  each stage to its milestones.
- `Status:` lines outside `adr/` were removed. Where a line carried a
  substantive sentence (docs 23–26, the fsync baseline) the sentence stays as
  plain text; the classification words are recorded here:

| File | Removed line |
|---|---|
| `ROADMAP.md` | Status: initial review complete. Continue subsystem-by-subsystem only when implementation reaches that subsystem. |
| `CODEX_HANDOVER.md` | Status: active handover document for continuing architecture supervision and implementation review. |
| `docs/23-pfs3-stage0-review.md` | Status: architecture review v1, completed before the first implementation milestone. Source-level study should continue when the corresponding AFS+ subsystem is implemented. |
| `docs/24-filesystem-comparison.md` | Status: design benchmark. This table compares architectural capabilities, not marketing claims. AFS+ entries marked Planned or Proposed are not implemented yet. |
| `docs/25-filesystem-wishlist.md` | Status: product/design exploration. Nothing in this document is automatically a 1.0 requirement unless promoted by an ADR. |
| `docs/26-debug-observability.md` | Status: required development architecture. Most facilities are runtime-optional and must have near-zero cost when disabled. |
| `docs/27-rust-implementation-strategy.md` | Status: implementation direction |
| `docs/28-virtual-images-and-viewports.md` | Status: developer-platform design |
| `docs/29-first-class-content-inspection.md` | Status: developer/security API design |
| `docs/30-portable-security-model.md` | Status: security architecture proposal |
| `docs/31-extreme-workloads.md` | Status: workload architecture and qualification plan |
| `docs/32-reflink-clone-semantics.md` | Status: epoch-1 design requirement for shared data extents |
| `testing/extreme-workload-benchmarks.md` | Status: required qualification design |
| `testing/security-model-conformance.md` | Status: required test plan before security-format freeze |
| `testing/aros-system-volume-qualification.md` | Status: required after Mountable Alpha-0 secondary-volume qualification |
| `testing/benchmark-contract.md` | Status: required benchmark policy |
| `testing/security-scanning-benchmarks.md` | Status: qualification requirement |
| `implementation/peer-review-prototype-plan.md` | Status: implementation guidance after external review |
| `implementation/fsync-intent-log-baseline.md` | Status: measurement report for architecture blocker 2 (prefix removed, sentence kept) |
| `proposals/*.md` | Status: proposal — for team review … (replaced by a "Target on acceptance" line; the review request is stated once in `proposals/README.md`) |

- The M06 detail that lived in the milestone cell (which platforms are
  qualified by which gate) is now
  [testing/aros-system-volume-qualification.md § 7](testing/aros-system-volume-qualification.md#7-qualified-configurations).
- `FILE_INDEX.md` and `SHA256SUMS.json` are deleted: no generator, no
  consumer, stale since the first commit; Git and the per-directory indexes
  replace them.

The `README.md` status section read as follows before it was reduced to the
five-row table (moved here verbatim from `README.md § Status`, commit
`624b6a8`):

> This repository now includes an executable Rust prototype, checker, portable
> VFS API, FUSE protocol adapter, packet-neutral AROS DOS adapter, a versioned
> AROS C/staticlib boundary, a cross-qualified native `DosPacket` translator,
> bounded trackdisk partition adapter and fully linked off-tree handler module.
> The host adapter and mount CLI can be built with
> `cargo build -p afsplus-fuse --features fuser-adapter --bin afsplus-mount`.
> Linux uses fuser's native mount path. On macOS, install macFUSE and build with
> `--features macfuse-mount`; the mount CLI selects macFUSE's user-space FSKit
> backend and the mountpoint must be an existing directory or a new direct child
> of `/Volumes`. The library is loaded at runtime, so ordinary workspace builds
> do not require a system FUSE installation. If macOS's File System Extensions
> switches are inert, use the diagnostic and reversible workaround in
> `docs/macos-fskit-activation.md`. Fuse-T's NFS transport is not a raw
> substitute for macFUSE's message channel; see ADR-040. The native AROS bridge
> and its cross-build qualification are documented in
> `docs/aros-native-bridge.md`, ADR-042 through ADR-060.
> `tools/check-hosted-aros-alpha0.sh` now qualifies a bidirectional Hosted
> MacAROS → macFUSE → Hosted MacAROS round trip on one checked image. Hosted
> intent-log replay and the cumulative post-bootstrap S1 system-volume pivot are
> also qualified. The handler now supports standard AROS `Mount SHUTDOWN`
> termination, restart through the retained device node and a final
> `Assign DISMOUNT`. AFS+ is distributed as an external `L:` handler plus
> DOSDriver and does not require acceptance of AFS+ into the upstream AROS
> source tree; an upstream build recipe remains optional. Genuine defects found
> in generic AROS interfaces are still fixed with focused regression-tested
> patches and proposed upstream. AROS qualification images use the versioned
> Unicode 16 case-insensitive, spelling-preserving namespace; general-purpose
> images remain configurable and default to case-sensitive lookup. The native
> Apple-AArch64 pre-hardware QEMU gate now loads the external handler from a
> versioned retained image, executes the same operation matrix and proves a
> clean dismount/unload. File-backed guest RAM lets the host extract and
> strictly check the mutated payload; all six modeled intent-log cuts also
> replay to their exact old/new state with zero pending records. The RAM block
> transport still makes no reset-durability or Apple-hardware claim; those
> remain explicit gates. Autonomous boot selection and the classic ports also
> remain later gates. Qualification is reported as three target platforms over
> four ordered stages: Hosted MacAROS, native MacAROS/Apple Silicon, Amiga
> 500/m68k emulation, then the physical A500. The emulator is the pre-hardware
> validation stage of the A500 target. `tools/check-aros-m68k-boot-fsuae.sh`
> now makes the first native m68k boot prerequisite machine-readable with
> matching official ROM, floppy and system-media hashes.
> `tools/check-aros-m68k-alpha0-fsuae.sh` now also qualifies the real external
> handler's Alpha-0 operation matrix and all six recovery cuts on both the
> M68020-or-newer reference profile and an A500-configured plain M68000
> profile. The latter rejects M68020 long-multiply instructions before boot and
> binds the patched LLVM library in its evidence. This deliberately makes no
> physical-A500, final constrained-memory-budget or hardware-performance claim;
> ADR-057 records the boundary. ADR-058 adds a measured 8-MiB emulator profile
> and proves that sequential shutdown/restart cycles reuse one DOS-loaded
> segment without material memory growth; 4 MiB does not boot the control AROS
> profile far enough to test AFS+. ADR-059 makes AROS fatal diagnostics an
> explicit verdict across Hosted, native-QEMU and FS-UAE gates, so a modal
> requester cannot be mistaken for a hang or a successful outer process.
> ADR-060 closes the Mountable Alpha-0 objective with
> `tools/check-mountable-alpha0.sh`: one checksummed composite result binds the
> portable APIs, real macFUSE round trip, Hosted/native operation matrices and
> twelve recovery cases. This completion makes no physical-hardware or format
> freeze claim.

Two facts from that paragraph found finished-state homes rather than this
journal: the mount-CLI build instructions are in
[README.md § Build, test and mount](README.md#build-test-and-mount), and the
external-handler distribution model is stated in
[README.md § What AFS+ is and is not](README.md#what-afs-is-and-is-not) with
[ADR-050](adr/ADR-050-external-aros-handler-lifecycle.md). The rule that
genuine defects in generic AROS interfaces are fixed with focused
regression-tested patches and proposed upstream is retained in `AGENTS.md`.

## 2026-08-31 — Mountable Alpha-0 closed

`tools/check-mountable-alpha0.sh` produced `result=PASS` with
`hardware_claim=none`: the portable VFS, FUSE protocol, intent-log and AROS
adapter suites, one real macFUSE/Hosted same-image round trip with clean
checkers at every boundary, six Hosted and six native replay cases (four exact
old states, two exact new states), no guest failure requester, and a
156-file checksum manifest that validated in full. The evidence set was
re-verified independently the same day in the gate's reuse mode. ADR-060
records the decision; M08 is complete; M06 stays partial because no physical
hardware was involved.
