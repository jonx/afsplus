# Notes

The project journal: what was decided, tried and delivered, newest first.
This is the only document that narrates. Everything else states the finished
state and links here for the story; see
[docs/DOCUMENTATION.md](docs/DOCUMENTATION.md) for the rules.

Entry format: `## YYYY-MM-DD — title`.

<!-- toc -->

- [2026-09-03 — Portable C overlays the durable namespace](#2026-09-03--portable-c-overlays-the-durable-namespace)
- [2026-09-03 — The per-file data policy becomes persistent](#2026-09-03--the-per-file-data-policy-becomes-persistent)
- [2026-09-03 — Portable C reads the durable log view](#2026-09-03--portable-c-reads-the-durable-log-view)
- [2026-09-03 — Portable C failures become replayable artifacts](#2026-09-03--portable-c-failures-become-replayable-artifacts)
- [2026-09-03 — The independent portable C reader starts executing](#2026-09-03--the-independent-portable-c-reader-starts-executing)
- [2026-09-03 — FSKit durability moves to write replies](#2026-09-03--fskit-durability-moves-to-write-replies)
- [2026-09-03 — Qualification order follows executable targets](#2026-09-03--qualification-order-follows-executable-targets)
- [2026-09-03 — macFUSE FSKit reopens the host fsync gate](#2026-09-03--macfuse-fskit-reopens-the-host-fsync-gate)
- [2026-09-03 — Logged data fsync reaches the portable Rust adapters](#2026-09-03--logged-data-fsync-reaches-the-portable-rust-adapters)
- [2026-09-03 — Intent-log writes and truncates survive nested recovery crashes](#2026-09-03--intent-log-writes-and-truncates-survive-nested-recovery-crashes)
- [2026-09-02 — Q1 and Q2 architecture blockers closed](#2026-09-02--q1-and-q2-architecture-blockers-closed)
- [2026-09-02 — Shared extents and reflinks qualified](#2026-09-02--shared-extents-and-reflinks-qualified)
- [2026-08-31 — Team board adopted](#2026-08-31--team-board-adopted)
- [2026-08-31 — Shared extents chosen as the next epoch-1 lot](#2026-08-31--shared-extents-chosen-as-the-next-epoch-1-lot)
- [2026-08-31 — Documentation restructured around one home per fact](#2026-08-31--documentation-restructured-around-one-home-per-fact)
- [2026-08-31 — Mountable Alpha-0 closed](#2026-08-31--mountable-alpha-0-closed)
- [2026-08-29 — Region allocator, bounded mount and shared COW trees](#2026-08-29--region-allocator-bounded-mount-and-shared-cow-trees)
- [2026-08-29 — Lesson from the first crash matrix, and the hardening list](#2026-08-29--lesson-from-the-first-crash-matrix-and-the-hardening-list)
- [2026-08-29 — First executable prototype](#2026-08-29--first-executable-prototype)

<!-- /toc -->

## 2026-09-03 — Portable C overlays the durable namespace

The independent C99 reader gained the namespace half of the intent-log view.
It now resolves and enumerates the final checkpoint-plus-log directory state,
chases rename chains without recursion, honors replacement, and keeps object
identity correct across hard-link deletion and rename. The same semantic pass
checks source/target preconditions, monotone create IDs and write/truncate
expected sizes before file bytes are exposed. Calls retain the 8-KiB
caller-owned scratch contract; an explicit cursor makes ordered iteration and
small-buffer retries possible without hidden allocation or global state.

The Rust-produced fixture grew from three to seven durable records and now
combines delete, write, truncate, create, rename chains and replacement.
Portable C matches both surviving oracle files and the final
case-insensitive namespace, while four valid-checksum mutations demonstrate
that semantic inconsistencies fail at the responsible record. A seventh AFZF
seed carries a successful namespace lookup through the deterministic
ASan/UBSan mutation and replay gate. The reader also learned
[ADR-065](adr/ADR-065-persistent-data-update-policy.md)'s per-file policy flag
and rejects the flag without its volume feature, keeping the Rust and C format
boundaries congruent.

## 2026-09-03 — The per-file data policy becomes persistent

ADR-062 had fixed the hybrid data-update semantics but deliberately left the
per-file opt-in unencoded, so the choice evaporated at every mount. ADR-065
(approved on board decision #63) assigns the encoding: object flag bit 1,
`OBJECT_FLAG_DATA_IN_PLACE`, protected against silent loss by the validated
object-flags namespace every implementation already enforces, plus `COMPAT`
bit 0 `org.aros.afsplus:data-policy` activated by mkfs profile with
shared-extent-style congruence — a flagged record on a volume without the
feature is corruption in the read path and the checker. The core gained
`set_file_data_policy`/`file_data_policy` (a metadata-COW record update) and
the VFS a `DATA_POLICY` capability with handle-based set/get; the write path
takes the in-place route when the record carries the flag, under unchanged
ADR-062 eligibility. One real bug surfaced while wiring it: layout staging
rebuilt object flags from scratch, so any write dropped the freshly set
policy bit — the persistence regression caught it immediately, and the bit
now travels through every layout rewrite. Seven persistence regressions plus
two VFS API tests cover remount survival, in-place stats, clearing, feature
refusal, planted-flag fail-closed on both sides, shared-block COW fallback
and the power-cut contract through the flag.

## 2026-09-03 — Portable C reads the durable log view

The independent C99 implementation no longer stops at the selected
checkpoint when a writer has acknowledged later fsync groups. With an 8-KiB
caller-owned scratch buffer it scans the bounded intent-log prefix, validates
record shape and sequence, content-verifies every referenced replacement block
and reports the first excluded tail slot and LBA. No replay is written to the
device.

The first view layer resolves create, write and truncate records into final
file sizes and bounded reads. Its cross-language fixture leaves three durable
records over a Rust checkpoint, then C reconstructs an existing file after
write-plus-shrink and a file that exists only in the log, byte-for-byte against
Rust-produced oracle files. Damaged data, sequence gaps and missing feature
contracts have exact regression expectations. Rename and delete explicitly
disable the file-data view until namespace identity is overlaid rather than
returning plausible but incomplete bytes.

The same decoder is now the sixth compact mutation seed. Parent and child
tree item counts are also compared during descent, closing an independent
review observation where an internally consistent parent over-claim could
otherwise hide reachable entries. Strict C99, sanitizer, analyzer, CMake and
m68k compilation remain one gate so future integrators get the same failure
coordinates as the in-tree build.

## 2026-09-03 — Portable C failures become replayable artifacts

The first portable-reader expansion substantially increased the amount of
untrusted C parsing, so its next lot hardened that surface before adding more
features. A host recorder now executes successful operations against a real
Rust-built image and writes only the blocks that were actually observed. Five
12–48-KiB packets retain checkpoint probing, object lookup, both extremes of a
303-entry directory and a complete directory-to-file read without copying or
carrying the complete disk image through every mutation.

Each packet receives 4,096 stable mutations under AddressSanitizer and
UndefinedBehaviorSanitizer in the ordinary repository gate. Every operation
header byte and the first 128 bytes of each observed block are tested both
with a broken checksum and with a recomputed CRC, so structural checks are not
hidden behind the checksum gate. Block-wide deterministic flips, overwrites,
multi-byte changes and truncations follow. The progress record is updated
before execution, so a crash names a seed and case number; the same case can
export an exact `.afzf` artifact and the replay tool reports symbolic operation
results plus the terminal stage and LBA. This turns parser failures into small
regression inputs an integrator can keep and exchange.

The harness also exports the standard libFuzzer callback. The installed Apple
command-line tools do not currently contain its link runtime, so that engine
is opportunistic rather than a hidden local prerequisite; the deterministic
sanitizer runner is the portable mandatory gate. The remaining M01 corpus is
listed by wire surface instead of being represented as a generic “fuzzing
open” checkbox.

## 2026-09-03 — The independent portable C reader starts executing

The first ADR-029 implementation slice added a C99 reader that does not link or
generate code from the Rust codecs. Caller-owned block I/O and one 4-KiB
scratch block are sufficient to validate the identification record, negotiate
the implemented feature summary, validate bounded geometry and structurally
select between both retained checkpoints. The public reader surface is
versioned before expansion so future profiles do not depend on native C struct
serialization.

Its cross-implementation gate formats and advances an image with Rust, compiles
the reader under strict C99 warnings, and asks C to select it. The same binary
injects a bad newest checkpoint, valid-checksum structural corruption, two bad
checkpoints, an impossible duplicate generation, a selected-state feature/root
mismatch, a bad identification checksum and a short scratch buffer, with
structured stage/LBA/per-slot diagnostics. The feature/root case proves that
selection does not hide corruption beneath a newer structurally valid
checkpoint by falling back. Sanitizer, CMake-example and available AROS-m68k
compiler gates catch host-language and integration faults; Rust's checker
validates the untouched source image. Object-tree traversal and file reads form
the next reader slice rather than being simulated through the Rust
implementation.

The next slice made that bootstrap useful without turning it into a second
host runtime. A 303-file Rust fixture forces object-map and directory descent
through internal nodes; C enumerates the root by ordinal, resolves a file
object, and reproduces its bytes through bounded 777-byte reads. A wire-valid
extent leaf over the same Rust-written data adds sparse-hole and extent-floor
coverage. Tree CRC, valid-checksum identity, generation/owner/level/range/count
invariants, typed object/directory/extent values and exact failing LBAs are now
checked along every accessed path. The API remains additive: the original
entry shape stays intact, new operations advertise capability bits, and all
storage remains caller-owned.

Independent review also caught that the checkpoint payload's reserved `flags`
word had no explicit reject-or-ignore rule in either implementation. Rather
than make the C reader silently stricter than Rust, the freeze choice is now
[Q10](implementation/open-questions.md) and must be closed during M14.

## 2026-09-03 — FSKit durability moves to write replies

The owner chose a scoped workaround instead of debugging the macFUSE/Apple
FSKit boundary. The macOS FSKit mount treats each delivered data write or
truncate as an implicit durability point and replies only after the AFS+ log or
checkpoint is durable. This preserves host `fsync` correctness even when the
transport omits `FUSE_FSYNC`, without changing the core, on-disk format,
host-neutral FUSE behavior or AROS adapters and without requiring the legacy
kernel extension.

The tradeoff is intentionally visible: small host writes lose cross-request
batching and can each pay the data-and-record barriers. A protocol regression
crashes immediately after successful write and truncate replies without an
`fsync` request. The real macFUSE mount then passed its syscall-boundary oracle:
host `fsync`, immediate second-descriptor inspection, clean unmount and checker
all succeeded without administrator authorization. The host benchmark contract
must label the fallback.

## 2026-09-03 — Qualification order follows executable targets

The owner reordered the four platform-validation stages so an unavailable
bare-metal target cannot block executable qualification. The sequence is
Hosted MacAROS, Amiga 500/m68k emulation, the physical A500, then native
MacAROS on Apple Silicon after that system exists. These are three target
platforms: the emulator and physical A500 are two stages for the same classic
target.

This ordering change does not weaken the reopened host `fsync` verdict. The
real FSKit mount proves that the syscall returns without delivering a FUSE
`FSYNC` request to AFS+, while direct VFS and FUSE-protocol tests exercise the
expected durability path. The defect is therefore below the AFS+ engine, in
the macFUSE-FSKit/Apple-FSKit path. The trace does not by itself assign the bug
between macFUSE's shim and Apple's FSKit plumbing; a stock macFUSE loopback
reproducer is the attribution gate.

## 2026-09-03 — macFUSE FSKit reopens the host fsync gate

Re-running the real mount after wiring the VFS log exposed a false-positive in
the Mountable Alpha-0 host gate. On macOS 26.6.2 (25G83) with macFUSE 5.3.3's
FSKit backend, both Rust `File::sync_all` (`F_FULLFSYNC`) and a direct
`fsync(2)` returned after dirty data reached the FUSE `WRITE` callback but no
FUSE `FSYNC` callback was delivered. Instrumentation distinguished `WRITE`,
`RELEASE` and `FSYNC`; only `RELEASE` arrived when the descriptor closed. For
two seconds after the syscall, a second descriptor on the image saw the same
checkpoint generation and zero pending intent records. The later rename or
unmount checkpoint had allowed the old round-trip test to pass.

The ignored real-mount test now inspects the live backing image immediately
after host fsync and requires either a replayable record or a newer checkpoint.
It intentionally fails on that FSKit combination, while the direct FUSE
protocol, VFS, AROS and C-bridge fsync tests pass. M08 is therefore partial
again. A safe FSKit-specific write-through workaround would add barriers to
every delivered write, so it requires measurement and an explicit policy
choice rather than being enabled silently; the preferable fix is for the host
stack to deliver FUSE `FSYNC` before returning from the syscall.

## 2026-09-03 — Logged data fsync reaches the portable Rust adapters

The portable VFS now routes existing-file writes and truncates through the
version-3 data-update window when the volume advertises that incompatible
feature. Reads and stats see the pending logical layout before publication;
`fsync` writes the bounded data/record barriers without advancing a checkpoint,
while oversized/full groups, namespace operations, reflinks and filesystem
sync publish the window through the ordinary checkpoint engine. Log-free and
older volumes retain their immediate checkpoint behavior.

The FUSE protocol, packet-neutral AROS Rust adapter and current C ABI bridge
inherit this path from the VFS. A trace test proves a successful VFS fsync
writes no checkpoint slot and that remount recovers its bytes; adapter tests
cover pre-fsync visibility, remount and checker cleanliness. The next
portability boundary is the independent portable-C filesystem implementation,
followed by real-device barrier measurements.

## 2026-09-03 — Intent-log writes and truncates survive nested recovery crashes

The accepted ADR-063 architecture now covers committed-file data changes in
the Rust core. Experimental record version 3 names freshly allocated COW
replacement blocks for writes and, when needed, one zero-tailed block for a
partial shrinking truncate. Data is flushed before the record; replay checks
the complete-block CRC, claims those exact blocks while they are FREE in the
base checkpoint and publishes the final layout through the ordinary COW
transaction engine. Multiple fsync groups on one file collapse into one final
metadata mutation without weakening their monotone-prefix recovery contract.

The new matrix covers old-or-new cuts around the live write, every write and
flush of recovery itself followed by another remount, write-plus-rename,
sparse growth, aligned and partial shrink, successive writes, and a reflink
split whose other owner retains the old bytes. Release workloads measured
2.200 writes/2.032 flushes per 200-byte logged append and 2.134/2.032 per 4 KiB
DB hot-set rewrite, versus 9.037/3.000 for checkpointed append. These are
memory-backend structural counts, not Apple or Amiga hardware results.

Review exposed a compatibility trap: an older version-2 scanner treats an
unknown record version as a torn tail. ADR-064 therefore assigns
`org.aros.afsplus:intent-log-data-updates` as a dependent INCOMPAT identity so
such readers reject the volume before silently losing an acknowledged fsync.
The numeric wire remains experimental; VFS/C parity, real-device flush testing
and the M14 format review remain open.

## 2026-09-02 — Q1 and Q2 architecture blockers closed

The shared-extent implementation made the private/shared write fork executable,
so Q1 was settled by measurement rather than preference. A runtime-only
private-in-place prototype was compared against full data COW on random 4 KiB,
database-hotset, append and reflink-first-write workloads. The large random
case fell from 26,972 to 8,000 device writes, 24,870 to 3,000 allocations,
21,936 to 2,000 retirements and 1,767 to one final extent. The database hot
set retained a smaller but material advantage; append and shared first-write
rows were identical because they correctly remained COW. Power-cut and
injected-error tests demonstrated the price directly: an old metadata
generation may expose old, new or torn overwritten bytes, and a returned error
does not imply byte rollback. ADR-062 therefore selected full COW by default
plus a persistent, explicit per-file private-in-place opt-in, never automatic
detection.

Q2's optimized re-run held the earlier result: a logged Git-style ref update
cost 2.152 writes and 1.032 flushes per operation, versus 10/3 for one
group-commit checkpoint and 26.992/6.998 for the original sequence. Review of
the executable record format caught an overclaim in the old report: version 2
can replay create/delete/rename and created-file content, but not writes or
truncates of existing files. ADR-063 accepted checkpoint COW, bounded group
commit and the intent log as the layered epoch-1 architecture while refusing
to freeze that namespace-only wire. Existing-file replay, the persistent Q1
policy representation, portable-C parity and real-device qualification remain
M14 gates rather than unresolved architecture choices.

## 2026-09-02 — Shared extents and reflinks qualified

The ADR-061 implementation completed the volume-wide reference tree, shared
copy-on-write writes, `CloneFile`, byte-range `CloneRange`, checker
reconstruction and corruption cases, and every-write/every-flush crash
matrices. Review caught and fixed the dangerous rc=2→1 case before closure:
removing one of two mappings deletes the reference record but never retires
the surviving file's blocks. A second review found a left-boundary prefetch
case that could leave adjacent equal-count records non-canonical; its
regression now exercises rc=2/rc=3→rc=2 merging.

When the original implementation session reached its limit, the remaining
range-clone lot was taken over in the shared checkout. The completed operation
shares every representable full block, privately copies partial boundaries,
keeps holes sparse and publishes both file maps with the reference tree in one
checkpoint. The same semantics are exposed through the portable VFS with
per-volume capabilities. The retained commands, revisions and limitations are
in [the completion report](implementation/shared-extents-completion.md).

## 2026-08-31 — Team board adopted

Coordination between agents and the owner moves to the shared
[agent-board](https://github.com/jonx/agent-board) (project `afsplus`).
`board init . --agents claude` wrote [.mcp.json](.mcp.json), the Claude Code
hooks under `.claude/` and the protocol block in [CLAUDE.md](CLAUDE.md);
[AGENTS.md § Team board](AGENTS.md#team-board) points to it. The board is the
live conversation; this file stays the repository's record.

## 2026-08-31 — Shared extents chosen as the next epoch-1 lot

With Mountable Alpha-0 closed, the next objective is the largest unimplemented
epoch-1 *requirement* rather than an open question: [ADR-027](adr/ADR-027-reflink-clones.md)
has been "Accepted as an epoch-1 format requirement" since the design phase, a
freeze gate depends on it, and nothing implemented it — the extent record
carried one flag and no reference state existed. ADR-027 itself records why
that cannot be deferred: sharing changes the allocator, checker, reclamation
and reverse-map invariants at once. It is also coupled to open question Q1,
because reflink-shared ranges always copy on write, so the write path must fork
on shared versus private — exactly where the data-policy decision lives.

[ADR-061](adr/ADR-061-shared-extent-references.md) selects the mechanism: a
volume-wide typed reference tree on the ADR-034 engine, keyed by physical run.
A first draft was revised after review found six contract gaps, two of them
genuine correctness holes: an operation on a flagged extent must partition by
overlap rather than look up its start, because peers split the underlying runs
independently; and although clone is not journalled, the existing journalled
delete and rename-replacement can remove a file holding shared extents, so
their replay has to maintain the counts. The review also refused the
checkpoint offset until the transitional inline region records — dead since
[ADR-035](adr/ADR-035-allocation-root-reserved-pool.md) but still decoded when
the allocation root is zero — were removed outright rather than left to
collide with a new field.

That review exposed a defect in these very rules: they declared ADRs immutable
from the moment they were written, which leaves no way to correct a draft that
review has rejected. The rule now says what it should have said — immutable
from acceptance, revisable while `Proposed`.

## 2026-08-31 — Documentation restructured around one home per fact

The repository's documentation is reorganised so that state, design,
procedure and history each have exactly one home
([README.md § Documentation map](README.md#documentation-map)), every document
is reachable from an index, and [`tools/check-docs.py`](tools/check-docs.py) enforces the mechanical
part of the contract (`make check-docs`).

Measured before the restructuring: 132 Markdown files (13,785 lines) outside
the code trees; 82 files carried a `Status:` line (60 ADRs and 22 others); 365
journey words ("now", "still", "remains", ...); 2 Markdown links between
documents against 118 backticked path mentions (9 of them not resolving from
the repository root); 59 documents referenced from nowhere; 31 files longer
than 150 lines, none with a table of contents; `FILE_INDEX.md` and
`SHA256SUMS.json` generated once in the initial commit and never again.

What moved where:

- [`implementation/milestones.md`](implementation/milestones.md) is the only status authority. [`README.md`](README.md)
  keeps a five-row summary; [`ROADMAP.md`](ROADMAP.md) carries the stage plan only and links
  each stage to its milestones.
- `Status:` lines outside `adr/` were removed. Where a line carried a
  substantive sentence (docs 23–26, the fsync baseline) the sentence stays as
  plain text; the classification words are recorded here:

| File | Removed line |
|---|---|
| [`ROADMAP.md`](ROADMAP.md) | Status: initial review complete. Continue subsystem-by-subsystem only when implementation reaches that subsystem. |
| `CODEX_HANDOVER.md` | Status: active handover document for continuing architecture supervision and implementation review. |
| [`docs/23-pfs3-stage0-review.md`](docs/23-pfs3-stage0-review.md) | Status: architecture review v1, completed before the first implementation milestone. Source-level study should continue when the corresponding AFS+ subsystem is implemented. |
| [`docs/24-filesystem-comparison.md`](docs/24-filesystem-comparison.md) | Status: design benchmark. This table compares architectural capabilities, not marketing claims. AFS+ entries marked Planned or Proposed are not implemented yet. |
| [`docs/25-filesystem-wishlist.md`](docs/25-filesystem-wishlist.md) | Status: product/design exploration. Nothing in this document is automatically a 1.0 requirement unless promoted by an ADR. |
| [`docs/26-debug-observability.md`](docs/26-debug-observability.md) | Status: required development architecture. Most facilities are runtime-optional and must have near-zero cost when disabled. |
| [`docs/27-rust-implementation-strategy.md`](docs/27-rust-implementation-strategy.md) | Status: implementation direction |
| [`docs/28-virtual-images-and-viewports.md`](docs/28-virtual-images-and-viewports.md) | Status: developer-platform design |
| [`docs/29-first-class-content-inspection.md`](docs/29-first-class-content-inspection.md) | Status: developer/security API design |
| [`docs/30-portable-security-model.md`](docs/30-portable-security-model.md) | Status: security architecture proposal |
| [`docs/31-extreme-workloads.md`](docs/31-extreme-workloads.md) | Status: workload architecture and qualification plan |
| [`docs/32-reflink-clone-semantics.md`](docs/32-reflink-clone-semantics.md) | Status: epoch-1 design requirement for shared data extents |
| [`testing/extreme-workload-benchmarks.md`](testing/extreme-workload-benchmarks.md) | Status: required qualification design |
| [`testing/security-model-conformance.md`](testing/security-model-conformance.md) | Status: required test plan before security-format freeze |
| [`testing/aros-system-volume-qualification.md`](testing/aros-system-volume-qualification.md) | Status: required after Mountable Alpha-0 secondary-volume qualification |
| [`testing/benchmark-contract.md`](testing/benchmark-contract.md) | Status: required benchmark policy |
| [`testing/security-scanning-benchmarks.md`](testing/security-scanning-benchmarks.md) | Status: qualification requirement |
| [`implementation/peer-review-prototype-plan.md`](implementation/peer-review-prototype-plan.md) | Status: implementation guidance after external review |
| [`implementation/fsync-intent-log-baseline.md`](implementation/fsync-intent-log-baseline.md) | Status: measurement report for architecture blocker 2 (prefix removed, sentence kept) |
| `proposals/*.md` | Status: proposal — for team review … (replaced by a "Target on acceptance" line; the review request is stated once in [`proposals/README.md`](proposals/README.md)) |

- The M06 detail that lived in the milestone cell (which platforms are
  qualified by which gate) is now
  [testing/aros-system-volume-qualification.md § 7](testing/aros-system-volume-qualification.md#7-qualified-configurations).
- `FILE_INDEX.md` and `SHA256SUMS.json` are deleted: no generator, no
  consumer, stale since the first commit; Git and the per-directory indexes
  replace them.
- `CODEX_HANDOVER.md` (955 lines, written 2026-08-29 at commit `c2c2ed3`) is
  split and deleted. Where each section went: §1 role, §2 context precedence,
  §19 things not to do, §20 files to read first, §21 use of the conversation,
  §22 expected interaction and §24 resume sentence → [`AGENTS.md`](AGENTS.md); §3 what
  AFS+ is and §4 language strategy → the review lens in [`AGENTS.md`](AGENTS.md), the
  facts themselves being [README.md](README.md), [ADR-028](adr/ADR-028-rust-reference-core.md)
  and [ADR-029](adr/ADR-029-dual-reference-implementations.md); §5 current
  implementation, §6 lesson from the first crash matrix, §7 hardening list and
  §23 next action → the dated entries below; §8 architecture blockers and §13
  developer-idea maturity → [implementation/open-questions.md](implementation/open-questions.md);
  §9 allocator experiment → the design is [docs/07](docs/07-allocation.md) and
  [ADR-035](adr/ADR-035-allocation-root-reserved-pool.md), the G1/G2/G3 reuse
  workload with its allowed and forbidden states is
  [testing/crash-testing.md](testing/crash-testing.md#block-reuse-across-generations),
  its metrics are in [testing/benchmark-contract.md § 1](testing/benchmark-contract.md#1-benchmark-dimensions);
  §10 crash-consistency rules → the reviewer checklist in [`AGENTS.md`](AGENTS.md), the
  design being [docs/08](docs/08-transactions-and-journal.md) and
  [ADR-021](adr/ADR-021-deferred-reclamation.md); §11 feature evolution →
  [docs/09](docs/09-feature-framework.md); §12 derived versus authoritative →
  [ADR-012](adr/ADR-012-catalog-derived.md), [ADR-013](adr/ADR-013-change-stream-bounded.md);
  §14 workload philosophy and §15 benchmark contract →
  [docs/31](docs/31-extreme-workloads.md), [testing/benchmark-contract.md](testing/benchmark-contract.md);
  §16 debuggability → [docs/26](docs/26-debug-observability.md) and the
  regression-scenario rule in [`AGENTS.md`](AGENTS.md); §17 portability rules →
  [docs/14](docs/14-paths-and-namespaces.md), [ADR-017](adr/ADR-017-namespace-outside-format.md);
  §18 security philosophy → [docs/30](docs/30-portable-security-model.md),
  [ADR-031](adr/ADR-031-portable-security-acls.md); the superseded ideas →
  [adr/README.md § Superseded directions](adr/README.md#superseded-directions).
  Two ADRs gained relation lines the text already implied: ADR-009 is amended
  by ADR-020, ADR-042 by ADR-056 and ADR-057.

The [`README.md`](README.md) status section read as follows before it was reduced to the
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
> [`docs/macos-fskit-activation.md`](docs/macos-fskit-activation.md). Fuse-T's NFS transport is not a raw
> substitute for macFUSE's message channel; see ADR-040. The native AROS bridge
> and its cross-build qualification are documented in
> [`docs/aros-native-bridge.md`](docs/aros-native-bridge.md), ADR-042 through ADR-060.
> [`tools/check-hosted-aros-alpha0.sh`](tools/check-hosted-aros-alpha0.sh) now qualifies a bidirectional Hosted
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
> validation stage of the A500 target. [`tools/check-aros-m68k-boot-fsuae.sh`](tools/check-aros-m68k-boot-fsuae.sh)
> now makes the first native m68k boot prerequisite machine-readable with
> matching official ROM, floppy and system-media hashes.
> [`tools/check-aros-m68k-alpha0-fsuae.sh`](tools/check-aros-m68k-alpha0-fsuae.sh) now also qualifies the real external
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
> [`tools/check-mountable-alpha0.sh`](tools/check-mountable-alpha0.sh): one checksummed composite result binds the
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
regression-tested patches and proposed upstream is retained in [AGENTS.md](AGENTS.md).

## 2026-08-31 — Mountable Alpha-0 closed

[`tools/check-mountable-alpha0.sh`](tools/check-mountable-alpha0.sh) produced `result=PASS` with
`hardware_claim=none`: the portable VFS, FUSE protocol, intent-log and AROS
adapter suites, one real macFUSE/Hosted same-image round trip with clean
checkers at every boundary, six Hosted and six native replay cases (four exact
old states, two exact new states), no guest failure requester, and a
156-file checksum manifest that validated in full. The evidence set was
re-verified independently the same day in the gate's reuse mode. ADR-060
records the decision; M08 is complete; M06 stays partial because no physical
hardware was involved.

## 2026-08-29 — Region allocator, bounded mount and shared COW trees

(Moved from `CODEX_HANDOVER.md` § 5 and § 23; the commits are `7a61c06`
through `35ec483`.)

Commit `7a61c06` completed the immediate hardening and the first Stage 6
allocator experiment: checkpoint selection is structural and never falls back
over corrupt state; same-generation slots are ambiguous; the negative
bad-ordering crash test is present; triple-buffered region descriptors and
three reserved generational slots per logical bitmap page break allocator
self-reference; retired blocks spend one generation in quarantine before
reuse; create-with-content, delete, the G1/G2/G3 reuse crash workload and
resource measurements are implemented. The first major proof after the
bootstrap prototype was not "allocation works" but "AFS+ can reuse storage
after deletes without any selectable checkpoint ever observing stale metadata
that points at newly reused content" — that invariant passed the modeled crash
matrix.

The continuation replaced the eager mount walk with bounded root loading.
Ordinary mount reads identification/checkpoints, the object-map root, root
object/directory-tree root and the retired-list root; descendant records are
decoded on access and allocation bitmaps are split across independently
checksummed pages, loaded as a mutation touches them through the selected
region descriptor. Clean pages that fail an allocation scan are evicted
immediately; the checker retains the exhaustive whole-volume view.

The Stage B1 continuation supports the proposed 262,144-block (1 GiB at 4 KiB)
region. The verifiable wire representation needs nine bitmap pages, not the
earlier rough estimate of eight. Three region-descriptor slots plus three slots
per bitmap page reserve 30 blocks (120 KiB, about 0.0114 %) per full region;
region 0 additionally contains ident and the two checkpoints. A small
transaction writes one dirty bitmap page and one descriptor before the
checkpoint. Cross-page allocation, corruption deferral/detection, quarantine
and power-cut behaviour are covered by executable tests. The multi-page design
deliberately does not put one record per bitmap page in the checkpoint: a
triple-buffered reserved region descriptor, itself selected by the
checkpoint's one record per region, binds the bitmap pages
([docs/07 § 3.1](docs/07-allocation.md)).

Core Scale-1 began with ADR-034 and the shared `AFST` node format: tree kind,
owner, level, strict binary keys, exact subtree item count and a counted
reference for every internal child, so an internal split can be planned from
the current page without reading every child. Transactional multi-upsert
copies a committed path once, keeps later changes in a write overlay, performs
balanced leaf/internal splits and grows the root; deletion merges or
redistributes underfull siblings and collapses one-child roots. A permuted
300-key test grows three levels, then deletes 299 keys in another permutation
and returns to one root leaf; a bad-level corruption test is executable. The
object-map adapter became authoritative (mkfs/checkpoints, bounded mount/stat,
mixed create/delete, checker ownership, crash matrices, a typed 1,001-entry
multi-page test), then the directory adapter (typed leaf at mkfs, bounded
lookup, exhaustive enumeration, 1,000-entry and end-to-end 300-entry
checked/remounted tests). Legacy object-map and directory codecs stayed as
transitional tests.

The namespace continuation removed the root-only API restriction: directory
lookup/enumeration, file creation/deletion, mkdir/rmdir, hard links and
same/cross-directory rename address parents by stable object ID. Rename
publishes both parent trees, parent records, the moved record and object-map
updates in one checkpoint, preserves object identity and rejects moves into
self or descendants; its every-write/every-flush matrix accepts only the
complete pre- or post-rename namespace. Link counts are transactional; the
first unlink preserves shared data and the final unlink retires it.

The typed extent-map adapter followed: small contiguous files keep the
zero-extra-tree direct representation; fragmented, sparse or preallocated
files set an experimental object flag and use an owner-bound `AFST` tree keyed
by logical block. `write_file_at` uses full data COW and supports writes beyond
EOF, `truncate_file` preserves zero-tail semantics, `preallocate_file` creates
zero-reading unwritten mappings, and final unlink retires both extent-tree and
mapped data blocks. A dedicated matrix proves sparse range writes recover to
exactly the pre- or post-transaction content.

The multi-upsert overlay retains every dirty node in RAM by default but has a
constrained mode with a 2/4/8-page final-image LRU that spills to
allocated-but-unreachable blocks; all three budgets pass a multi-level mutation
test, and a 100,000-key eight-page structural qualification passes (about 8.1 s
optimised, 53 s debug, so it is an explicit ignored scale test). This did not
close the classic-memory gate: decoded recursive ancestors and the split peer
stayed outside the measured staged-image budget.

ADR-035 fixed the allocation-root self-reference direction: `AFST` nodes in a
permanently allocated `3N` pool for an `N`-node fixed-topology region tree,
leaving a complete writable generation while two checkpoints stay selectable.
`TreeAllocator` decoupled the COW engine from ordinary free-space allocation;
inline checkpoint records became empty. A 1 TiB geometry bulk-build creates
1,024 typed records; a sparse 1 TiB image formats, bounded-mounts, performs two
small commits and runs the exhaustive checker in about 3.51 s on the
Apple-Silicon/APFS development host (commits about 18.6/12.0 ms, checker about
2.34 s), with host physical allocation between about 112 MiB and 2.13 GiB, so
the test uses a relative < 1 % sparse bound. Allocation-root publication
upserts only dirty region records (at most three tree nodes per measured
commit); the 145-region boundary forces a two-level allocation root and has an
exhaustive crash matrix; checker bitmap equality compares a sparse accounted
set against set bits byte-wise.

The next actions recorded at that point were: prototype multi-page region
bitmaps and larger regions against the one-page baseline, and measure the
fsync checkpoint path before deciding whether an auxiliary durability log is
justified — both done later the same week
([fsync-intent-log-baseline.md](implementation/fsync-intent-log-baseline.md),
[ADR-036](adr/ADR-036-reclaim-queue.md), [ADR-037](adr/ADR-037-intent-log.md)).

## 2026-08-29 — Lesson from the first crash matrix, and the hardening list

(Moved from `CODEX_HANDOVER.md` § 6 and § 7.)

Because the bootstrap allocator never reused blocks, an incorrectly ordered
commit could sometimes be hidden by doing a full reachable-state validation at
mount and falling back to the old checkpoint. That is dangerous: the crash
harness appears to prove ordering correctness while mount is repairing around
an invalid commit protocol. `validate_checkpoint_reachable()` also walked
every object reachable from the checkpoint; used as the ordinary selection
path it would turn mounting a volume with millions of files into a full-volume
scan. The design direction that followed — bounded structural checkpoint
selection at mount, full reachable validation only in `afsplus-check`, shadow
verification, the test harness and explicit recovery modes; torn or
CRC-failing checkpoints as the only legitimate fallback — is the rule the
reviewer checklist in [`AGENTS.md`](AGENTS.md) carries.

The hardening list executed before the allocator work: separate bounded
checkpoint selection from full reachable validation and add the negative
mis-ordered-commit crash test; make `Timespec::read()` and every public decode
helper reject short buffers instead of panicking; checked arithmetic for
generation, next free block, next object ID and range arithmetic; two distinct
checkpoint slots claiming the same generation are ambiguous, not tie-broken;
the power-cut model is described exactly as what it enumerates (all
full-write subsets plus representative torn writes of the modeled tail);
directory entries enforce `entry.key == comparison_key(entry.name)`; an object
in the authoritative object map with zero namespace references fails
validation until an explicit orphan model exists.

## 2026-08-29 — First executable prototype

(Moved from `CODEX_HANDOVER.md` § 5; commit `74b1410`.)

Steps 1–5 of
[implementation/peer-review-prototype-plan.md](implementation/peer-review-prototype-plan.md)
became executable and green: 29 tests, zero clippy warnings, about 3,592
lines, `afsplus-format` building as `no_std + alloc` on
`aarch64-unknown-none`. `afsplus-format` held the prototype wire codecs
(explicit little-endian, no native-struct serialisation, CRC32C, a 32-byte
common metadata header with type, version, owner, generation, payload length
and checksum, bounds-first decoding, a normalisation-preserving directory
entry shape with an identity comparison-key encoder). `afsplus-block` held
the block provider plus memory, sparse file, trace/accounting, deterministic
fault and power-cut recording backends. `afsplus-core` held mkfs, the
identification record, alternating checkpoints A/B, bootstrap bump allocation
from a checkpoint high-water mark (scaffolding that never reused blocks), the
root object, root directory and object map, the first COW metadata
transaction (create empty file: four fresh metadata blocks, flush, alternate
checkpoint, flush) and the shared reachable-state validator. `afsplus-check`
was a verify-only checker reporting both checkpoint slots as text and
versioned JSON.
