# Developer Harness

> **ADRs:** none · **Spec:** none ·
> **Tests:** none · **Milestones:** M05

<!-- toc -->

- [Goal](#goal)
- [Stage A finite acceptance](#stage-a-finite-acceptance)
- [Host-native inner loop](#host-native-inner-loop)
- [Scenario format](#scenario-format)
- [Failure minimization](#failure-minimization)
- [Differential tests](#differential-tests)
- [Tiny-cache matrix](#tiny-cache-matrix)
- [Crash-point exploration](#crash-point-exploration)
- [Shadow reader](#shadow-reader)
- [Block ownership assertions](#block-ownership-assertions)
- [Remote AROS target mode](#remote-aros-target-mode)
- [Rule](#rule)
- [Bounded partition views](#bounded-partition-views)
- [Bounded overlay branches](#bounded-overlay-branches)
- [Persistent block-operation traces](#persistent-block-operation-traces)
- [Bundle integrity and publication](#bundle-integrity-and-publication)
- [Bounded semantic scenario execution](#bounded-semantic-scenario-execution)
- [Integrated semantic bundles and minimization](#integrated-semantic-bundles-and-minimization)
- [Selected crash bundles](#selected-crash-bundles)
- [Checker-bound replay verdicts](#checker-bound-replay-verdicts)
- [Integrated tree-cache profiles](#integrated-tree-cache-profiles)
- [Cache-bound semantic bundles](#cache-bound-semantic-bundles)
- [Comparing a rebuilt runner](#comparing-a-rebuilt-runner)
- [Preserving and restoring working sources](#preserving-and-restoring-working-sources)
- [Retaining registry dependencies for a cold build](#retaining-registry-dependencies-for-a-cold-build)
- [Copied host toolchain and SDK qualification](#copied-host-toolchain-and-sdk-qualification)
- [Automated host reconstruction](#automated-host-reconstruction)
- [Common checkpoint-tail flight recorder](#common-checkpoint-tail-flight-recorder)
- [Category selection and live diagnostics](#category-selection-and-live-diagnostics)
- [Core API call spans](#core-api-call-spans)
  - [Publication-family observation equivalence](#publication-family-observation-equivalence)
- [Object-map observation](#object-map-observation)
- [Deferred-window observation](#deferred-window-observation)
- [Mount and recovery observation](#mount-and-recovery-observation)
- [Format observation](#format-observation)
- [Verification observation](#verification-observation)
- [Pre-tail and read-path observation](#pre-tail-and-read-path-observation)
- [Publication-family execution evidence](#publication-family-execution-evidence)
- [API guard execution](#api-guard-execution)
- [Diagnostic mechanism qualification](#diagnostic-mechanism-qualification)
- [API and window replay bundles](#api-and-window-replay-bundles)
- [Object-map replay bundles](#object-map-replay-bundles)
- [Captured snapshot replay bundles](#captured-snapshot-replay-bundles)
- [Subsystem and lifecycle replay bundles](#subsystem-and-lifecycle-replay-bundles)
- [Linked namespace replay bundles](#linked-namespace-replay-bundles)
- [Selected-category and live-delivery bundles](#selected-category-and-live-delivery-bundles)
- [Internal diagnostic bundles](#internal-diagnostic-bundles)

<!-- /toc -->

## Goal

A filesystem bug should be reproducible from a small artifact set rather than from a verbal description of what happened on a machine.

Every failing test run should be able to produce:

```text
run.json
operations.afstrace
block-io.afstrace
flight-recorder.bin
start.img
result.img
fault-model.json
expected.json
actual.json
```

## Stage A finite acceptance

The executable-core stage requires these host-level outcomes:

- A workspace with format, device, core and checker components; disk semantics
  are independent of platform namespace syntax.
- Memory and sparse-file devices, deterministic tracing/faults/cuts, and bounded
  partition views and isolated overlay branches with explicit flush semantics.
- Formatting, mutation, checkpoint publication, simulated interruption, remount
  and independent exact-state verification, including a deliberately broken
  ordering negative control.
- The [finite accounting harness](benchmark-contract.md#stage-a-accounting-acceptance).
- Retained source, dependencies, qualified host tools and semantic artifacts
  sufficient for fresh reconstruction, replay and failure minimization.
- Bounded core diagnostics correlating API calls, deferred operations and commit
  attempts across executable subsystem paths, with export/replay, loss accounting
  and failure comparisons against unobserved execution.
- The 2/4/8/unlimited cache matrix across executable mutation/publication families,
  with spill, refusal, recovery and exact-image invariants.
- Malformed-input and property coverage of executable codecs and operation
  families, with deterministic seeds, independent semantic checks and preserved
  failure artifacts.

New executable families extend their corresponding regression matrix. Proposed
wire structures receive qualification when their implementation is admitted.
Platform adapters, cross-host reconstruction and physical-provider evidence feed
Stages C/D/F; epoch freeze, sustained application benchmarks and native resource
budgets retain their own gates. Completion of the Stage A host scope does not
satisfy those later requirements. The
[scoped status registry](../implementation/milestones.md#scoped-stage-acceptance)
owns each disposition and its evidence.

## Host-native inner loop

The default development loop runs against a regular host file as the block device:

```text
build libafsplus
create/reset fixture image
run semantic operation script
inject requested fault
simulate crash if requested
reopen NO_CHANGES
validate checkpoint selection
run invariants
compare expected namespace
archive traces on failure
```

Target command shape:

```text
./afsptest run scenarios/rename-crash.yaml
./afsptest replay artifacts/failure-123/
./afsptest minimize artifacts/failure-123/
```

## Scenario format

A scenario should describe semantic actions, not internal block numbers:

```yaml
volume:
  size: 1GiB
  block_size: 4096
  cache_pages: 4
  deterministic: true

steps:
  - mkdir: /src
  - create: /src/a
  - write:
      path: /src/a
      bytes: 65536
  - fsync: /src/a
  - rename:
      from: /src/a
      to: /src/b

fault:
  power_cut_after_event: CHECKPOINT_WRITE_BEGIN

expect:
  valid_states:
    - /src/a
    - /src/b
  invariants: full
```

Exact syntax is not frozen.

## Failure minimization

The harness should support delta-debugging a semantic operation sequence:

1. reproduce the failure
2. remove ranges of operations
3. replay with identical deterministic settings/fault model
4. retain the smallest sequence that still fails

A 20,000-operation fuzz failure should ideally become a 6-operation regression test.

## Differential tests

Where useful, run the same high-level behavior through:

- checkpoint-COW prototype
- redo-journal prototype
- different cache sizes
- different allocation policies

The harness compares user-visible results and invariant outcomes, while performance instrumentation compares write amplification and latency.

## Tiny-cache matrix

Every mutation suite runs at minimum with:

```text
2 metadata pages
4 metadata pages
8 metadata pages
normal cache
```

This intentionally stresses cache pinning and stale-reference bugs.

## Crash-point exploration

Instead of manually selecting every failure point, the harness can enumerate trace events marked crash-safe boundaries:

```text
for each crashable event in operation:
    run from same image
    crash at event
    reopen
    validate
```

This produces exhaustive crash-state coverage for bounded operations.

## Shadow reader

The writer under test is not allowed to validate only its own cached structures.

After commit, a fresh read-only `libafsplus` instance reopens the image from bytes and verifies the committed namespace and metadata.

Later, an independent minimal-reader implementation can be added as a stronger differential oracle.

## Block ownership assertions

When reverse-map support is enabled, validation checks both directions:

```text
object/extent -> physical blocks
physical blocks -> semantic owner
```

Disagreement is an immediate failure.

## Remote AROS target mode

Once `afsplus.handler` exists, the same semantic suite should run against AROS.

The target adapter should:

- reset or attach a disposable image/device
- execute filesystem API operations
- stream flight-recorder events to the host
- return structured results
- trigger reboot/power-cut tests where the platform harness permits it

For Macaros Native, the M5-to-M1 development control path can be used to collect traces and reboot the target after intentional filesystem/kernel crashes.

## Rule

A filesystem bug fixed without a reproducible regression scenario is not considered fully fixed unless reproduction is genuinely impossible.

## Bounded partition views

[ADR-096](../adr/ADR-096-bounded-block-slices.md) defines a fixed, nonempty
parent-block interval with subtraction-first capacity admission. Reads and writes
validate logical bounds and exact buffer size before translating the address;
flush forwards the parent barrier and its errors. Nested views allocate no
additional transfer buffer and consuming a view returns its parent.

Run `cargo test -p afsplus-block` for translation, neighboring sentinels, nested
views, invalid ranges/buffers and forwarded failure checks. Run
`cargo test -p afsplus-check --test sliced_volume` for format, sparse write,
rename, truncate, sync, remount and exhaustive checking within a populated parent
image. Every surrounding block must retain its sentinel contents. These tests
use memory images and establish no physical partition or device qualification.

## Bounded overlay branches

[ADR-097](../adr/ADR-097-bounded-memory-overlay-branches.md) defines memory-only
forks over a stable shared base. Caller-selected limits cover live branches and
aggregate replacement entries, including copied fork indexes. Payloads are shared
until rewritten. Base reads propagate errors; branch writes and flushes never
write or flush the base. Host-process loss is outside this memory provider's
storage scope.

Run `cargo test -p afsplus-block` and
`cargo test -p afsplus-check --test overlay_volume`. Require:

- Exact fork isolation and immutable base contents through filesystem namespace
  changes, sparse writes, truncates, checker passes and remount.
- Full fork admission before index allocation, rollback on failed admission,
  rewriting existing entries at capacity, and capacity return on branch drop.
- Refusal without base I/O for invalid bounds/buffers; base-read error propagation.
- Fork index/payload measurements equal across small and large logical images
  for the same sparse edits, with shared base/payload identity and no full-image copy.
- Overlay cut states equal the independent memory-image oracle byte for byte,
  including repeated-block writes, completed barriers, write-loss subsets and
  sampled tears. Every publication cut preserves exact old/new namespace contents.
- Exhaustion and callback errors return incomplete enumeration explicitly and
  release unretained branches. Retaining callback branches consumes family limits.

Payload/index measurements exclude allocator overhead, shared-base memory and
total process RAM. Full resource accounting and persistent replay artifacts keep
separate stage gates. A persistent scratch provider needs crash-safe payload/index
publication before it can replace the memory provider for larger workloads.

## Persistent block-operation traces

[ADR-098](../adr/ADR-098-bound-block-replay-traces.md) specifies version-1 trace
geometry, base identity, write/flush records and whole-artifact SHA-256. The
[codec](../crates/afsplus-check/src/replay_trace.rs) validates caller bounds and
all serialized input before returning operations. Base verification reads the
admitted logical image and never writes it; this exhaustive operation belongs
to offline replay qualification and never to normal mount.

Run `cargo test -p afsplus-check --test replay_trace`. Require deterministic
serialization and persisted file readback, Python-independent geometry/operation
and digest checks, every truncated prefix and single-byte corruption, resealed
invalid headers/tags/LBAs, trailing data, caller admission limits, modified-base
refusal and zero writes during verification. Feed decoded operations to the
overlay cut enumerator and compare every state with the original memory oracle.

Wire-byte and operation-count caps jointly bound decoder allocations, including
operation-vector overhead. Base-block and block-size caps bound exhaustive hash
work and its single transfer buffer. Callers must retain a stable base between
identity verification and replay, such as an exclusively owned memory provider;
a verified digest cannot prevent later external changes to a host file.

This trace carries block I/O and base identity. The enclosing bundle must also
bind the source revision, semantic scenario, fault selection and expected/actual
state, and publish a completion record only after all artifacts are durable.
Serialization alone does not close semantic replay, minimization or the complete
failure-artifact gate.

## Bundle integrity and publication

[replay-bundle.py](../tools/replay-bundle.py) publishes the fixed artifact roles
from this harness with size/digest descriptors in a versioned completion manifest.
Run `python3 tools/test-replay-bundle.py` for deterministic publication and a fresh
verification process, every artifact-write and synchronization failure boundary,
existing-name collision, corrupt/missing/oversized/symlink artifact refusal and
manifest role/version/size admission. These inject I/O errors and do not simulate
physical host-storage power loss.

The output directory is created exclusively. Files are synchronized before
completion publication, followed by the bundle directory and its parent. The
completion link cannot replace an existing name. Late synchronization failure
returns an error even when a complete manifest is readable. Verification captures
all admitted artifact bytes and validates their digests without executing their
contents or modifying original files. It establishes integrity/completeness only;
source identity, scenario execution and expected-state comparison belong to the
semantic runner described in [ADR-099](../adr/ADR-099-semantic-replay-bundles.md).

## Bounded semantic scenario execution

[replay-scenario.py](../tools/replay-scenario.py) admits a
[versioned JSON scenario](#cache-bound-semantic-bundles) and compiles its
operation list to the corresponding fixed ASCII protocol consumed by
[the memory runner](../crates/afsplus-check/src/scenario.rs). Names and data are
hex fields; object labels resolve only inside the fixture. JSON admission checks
label dependencies and kinds before compilation. The Rust parser independently
checks syntax, geometry, operation count and payload/range limits; direct protocol
label errors are execution failures with retained evidence.

The profile admits 4 KiB blocks, at most 65,536 blocks, 1,024 operations, 1 MiB
of operation payload and 16 MiB logical file ranges. JSON admission additionally
counts expected file contents in its 1 MiB aggregate. These are harness admission
limits, not filesystem format limits. The default captured block log admits
65,536 operations and 64 MiB of payload across all remounts. Lower caller caps
exercise explicit refusal. Admission and fallible log/payload reservation precede
each device mutation; successful operations form an exact replayable prefix.

A failed filesystem operation or remount returns its operation index and error
with the base, result and captured block log. Formatting failures return an error
before a run exists. Exact namespace inspection checks file/directory paths and
contents, rejects unsupported kinds or repeated objects, and admits at most 1,024
entries, depth 64 and a caller-selected aggregate content budget. Inspection runs
on an owned memory image; callers preserve the original result by passing a copy.

Run `python3 tools/test-replay-scenario.py` and
`cargo test -p afsplus-check --test scenario`. Require exact create/write/truncate/
rename/unlink/directory/sync/remount results, full block-log reconstruction,
retained failure prefixes, malformed-input refusal and observation-budget refusal.
These checks cover runner and codec components. Complete bundle replay additionally
requires source/fault binding, flight-record export, fresh-process semantic
comparison and failure-preserving minimization from
[ADR-099](../adr/ADR-099-semantic-replay-bundles.md).

## Integrated semantic bundles and minimization

Build the private memory runner and use the JSON schema admitted by
[replay-scenario.py](../tools/replay-scenario.py):

```sh
cargo build -p afsplus-check --bin afsplus-scenario
python3 tools/afsptest.py run scenario.json /private/tmp/new-afsplus-bundle
python3 tools/afsptest.py replay /private/tmp/new-afsplus-bundle
python3 tools/afsptest.py minimize /private/tmp/failing-afsplus-bundle /private/tmp/reduced-afsplus-bundle --max-runs 128
python3 tools/test-afsptest.py
```

[afsptest.py](../tools/afsptest.py) combines the admitted scenario, memory runner,
block-trace codec and exclusive bundle publisher. The `semantic-no-cut-v1` profile binds an explicit version-1 `no-cut` fault
model. The [selected-crash profile](#selected-crash-bundles) binds an anchored
power-cut model; unsupported fault models are refused before execution. Replay verifies every artifact digest,
trace checksum, base-image digest, geometry, reconstructed result and semantic
event ranges, then reruns the scenario in a fresh runner process and compares
all artifact bytes. Images are private memory fixtures. The input bundle is
read-only. A deliberately wrong expected state produces a failure bundle and
reproduces as failure, never as a successful qualification.

Exit codes are 0 for semantic success, 2 for a captured/reproduced semantic
failure, and 1 for admission, execution-infrastructure or binding failure.
The runner is selected by the caller's `--runner` option, never bundle content.
Exports admit image size before execution; runner input, output roles and framed
lengths have bounded profiles. Output goes through a private temporary spool
with a 120-second runner timeout, then the bundle's file/aggregate admission.
Default per-artifact admission is 64 MiB and aggregate admission is 256 MiB;
explicit larger limits do not enlarge the Rust runner's own profile.

Run metadata records the observed Git revision, a digest of tracked and unignored
working files (including symlink targets and modes), and the runner executable's
digest. Source/runner changes during execution are rejected. These bind the
observed source and selected executable, not an attestation that the executable
was built from that source. The matching source tree and executable must be
retained for reproduction; [source packages](#preserving-and-restoring-working-sources)
preserve the source identity independently of build-provenance evidence.

The semantic flight format is `AFSFLT01`, a little-endian event count followed
by 29-byte records: operation index (u32), first and exclusive-end block-operation
indices (u64 each), resolved object identity (u64; zero where inapplicable), and
success (u8). It captures successful and failed semantic operations across
remounts and maps them to the recorded write/barrier stream. It does not replace
transaction-internal diagnostics or capture block-read activity.

Minimization first reproduces the retained failure. It removes contiguous
operation ranges and rejects invalid label dependencies before execution.
Operation failures require the same operation record and error; observation
failures require the same error; expected-state failures require the same exact
set of expected/observed differences. Parser or infrastructure failure cannot
become a replacement reproducer. The execution budget is explicit (1–4096
candidate runs). Exhaustion publishes the best verified reduction with
`budget_exhausted`; it makes no minimality claim. Reduction metadata carries the
parent scenario digest and evaluation count. The original bundle is preserved
and the reduced result is published exclusively as another complete bundle.

To evaluate a fix or another implementation revision, run the retained scenario
and fault inputs into a new bundle:

```sh
python3 tools/afsptest.py run original/operations.afstrace new-result --fault original/fault-model.json
```

This records an independent result with the selected runner and observed source
identity. Strict replay verifies the original environment and artifacts.

The focused gate checks fresh-process success and failure replay, exact input
immutability, altered source/fault/base/trace/event refusal, operation failure
preservation, pre-run image admission, removal of irrelevant operations with the
same failure signature, and explicit budget exhaustion. Transaction-internal flight diagnostics, portable source/build reconstruction
and cache/resource qualification are additional stage gates. Selected crash
replay uses the profile below; publication-family coverage needs its own oracles.

## Selected crash bundles

The `semantic-power-cut-v1` profile takes `--fault fault.json` on the run
command. A fault file has this exact shape:

```json
{"version":1,"kind":"power-cut-v1","operation":0,"offset":1,"variant":0}
```

The operation index selects a semantic event. The offset selects a cut within
that event's half-open block-log range, allowing both boundary cuts. The variant
uses the ordering of the [power-cut model](../crates/afsplus-block/src/powercut.rs):
all subsets of the unflushed writes, followed by in-order single-write tears at
64, 2048 and 4064 bytes. The 4 KiB runner profile admits at most 12 unflushed
writes and refuses a larger tail, an absent operation, an out-of-range offset or
an unavailable variant. Variant zero with an empty tail selects the completed
durable prefix. This is the stated simulation model, not every physically
possible storage failure.

The runner records a successful baseline scenario before selecting its crash
prefix. Its retained block trace and semantic flight describe that baseline;
the result image and remounted namespace/content observation describe the
selected crash. A baseline that fails to finish recording cannot qualify a
selected crash. [crash_replay::select](../crates/afsplus-check/src/crash_replay.rs)
selects one state directly, avoiding enumeration of unrelated images. The Rust
gate compares every selected variant with the existing enumerator, including
repeated LBAs, 512-byte/4-KiB devices, every cut of the fixture, and invalid
geometry/tail/selection refusal.

The Python verifier independently derives the durable prefix and selected
subset/tear from the retained trace and event range, verifies the result image
before semantic re-execution, and compares the regenerated bundle byte-for-byte.
Expected state is supplied explicitly in the scenario. A mountable image that
violates that state is a failure. The selected-crash tests require the old empty
namespace at the first unflushed write (loss/full/three tear variants), and exact
committed file contents at the completed transaction boundary.

Minimization never deletes the anchored operation. Removing earlier operations
remaps its index; the operation-local offset and variant stay fixed. Acceptance
also requires the same unflushed-tail count and variant kind, as well as the
same observable failure signature. A changed or inadmissible cut cannot replace
the original fault. Original bundles are preserved, and reduced crash bundles
must pass the independent verifier and fresh replay.

Run `cargo test -p afsplus-check --test crash_replay` and
`python3 tools/test-afsptest.py`. These qualify selection and bundle transport;
each publication path needs its existing allowed-state crash oracle and required
cache/resource variants before its finite gate can close. The bundle verdict combines remounted namespace/content, trace consistency
and the [raw/recovered checker reports](#checker-bound-replay-verdicts).

## Checker-bound replay verdicts

The `AFSOBS02` runner observation and version-2 `actual.json` retain two full
version-5 checker reports: `raw_check` describes the selected result image before
recovery; `recovered_check` describes the exact owned volume used for remounted
namespace/content observation. Raw checking is read-only. Mount/recovery and the
second check run on a memory copy, preserving the retained result image.
Each report includes its checkpoint selection, volume summary, warnings and
errors. If mount fails, the recovered report is absent and the inspection error
is retained.

Success requires both checker reports to be clean, a successful scenario and
inspection, and an exact expected namespace/content match. Checker cleanliness
uses the checker's error policy; warnings are retained without silently becoming
errors. A missing report, unsupported schema or verdict that contradicts its
error array cannot stand in for a clean check. Older observation payloads without
checker evidence are rejected by this runner profile.

Failure minimization preserves structural errors for both views and any
inspection error. A reduction cannot replace a checker failure with merely
matching file contents. Replaying a bundle regenerates and compares the complete
reports, including warnings and summaries, alongside every other artifact.

The Rust scenario gate creates a synchronized image, marks a reachable object
block free and a different free block allocated, then reseals the bitmap. Counts
remain consistent and namespace reads succeed, but both full checker views must
reject the ownership defect. It also verifies clean raw/recovered views and
explicit namespace-output budget refusal. Python gates require both reports,
reject contradictory verdicts and preserve structural failure signatures.
These tests establish verdict integration; the checker's full wire-surface
coverage is tracked by its [corruption corpus](corruption-corpus.md).


## Integrated tree-cache profiles

The [family matrix](../crates/afsplus-check/tests/tiny_cache_matrix.md) maps
executable mutations to profile, fault and recovery oracles. Its
[added tests](../crates/afsplus-check/tests/tiny_cache_matrix.rs) exercise exact
live and retained state across 2/4/8/unlimited profiles; the matrix states
each fixture limit and separates small-tree semantics from actual eviction.

[The runtime policy](../docs/27-rust-implementation-strategy.md#10-transaction-tree-resource-policy)
applies at mount, including replay, and at every transaction allocator. Run:

```sh
cargo test -p afsplus-check --test cache_profiles -- --nocapture
cargo test -p afsplus-core allocation_cache_keeps_spilled_nodes_across_checkpoint_rotation
cargo test -p afsplus-core constrained_
```

The [integrated matrix](../crates/afsplus-check/tests/cache_profiles.rs) uses
2/4/8 staged pages and the unlimited profile. A 192-file wide-name batch forces
actual spills in every constrained profile. Creation, deletion and remount must
preserve the exact namespace and bytes. A separately fsynced, uncheckpointed
192-file window must replay under the chosen mount profile and be idempotent.
Commit bytes and flushes must equal observed device requests, including early
spills. Zero-page configuration and changes during an open window must refuse.

A directory-split fixture must produce a real spill with two pages. Every modeled
write/flush cut enumerates the existing whole-write subsets and representative
tears, requiring a clean raw checker and exactly the old or new namespace. Four
injected early write failures must preserve the old view and allow a successful
retry with exact contents. These checks use memory devices exclusively.

The allocation-root rotation regression spreads dirty records across several
leaves and compares both cached node sets with fresh traversals after three
checkpoints. Both selectable checkpoints must pass full invariant sweeps.
The snapshot profile matrix checks shared survivors, historical bytes after
batched deletion and remount, both checkpoint views and physical-request
accounting under all four profiles.

Use the [batch resource workload](benchmark-contract.md#tree-cache-batch-measurements)
for per-profile heap and I/O reports. This matrix proves the named paths; other
mutation families need their own forced-eviction, error, crash and low-space
cases. Bulk tree builders, total-volume memory caps, shared-cache pinning and native profiles have
separate qualification requirements. Retain source/profile identity and failure
artifacts with each broader scenario's semantic oracle.


## Cache-bound semantic bundles

Scenario version 2 requires `volume.tree_cache_pages` to be the integer `2`, `4`
or `8`, or the string `"unlimited"`. The same policy applies to initial mount,
every explicit remount, and recovery/inspection of the selected result image.
The [scenario compiler](../tools/replay-scenario.py) rejects missing profiles,
unknown values, boolean or floating-point aliases and cross-version fields.

| Scenario JSON | Command wire | Observation wire | Actual JSON | Resource policy |
|---|---|---|---|---|
| version 1 | AFSPSC01 | AFSOBS02 | version 2 | implicit unlimited |
| version 2 | AFSPSC02 | AFSOBS03 | version 3 | explicit qualified profile |

The version-2 `format` line appends the canonical profile token after the four
geometry values. AFSOBS03 adds `cache-pages TOKEN` immediately after its version
line; the remaining checker/outcome/namespace records follow AFSOBS02. Version-3
actual JSON adds `cache_pages`. Successful inspection confirms the effective
mounted policy. On mount failure the record retains the attempted profile and
the failure verdict; it establishes no resource qualification.

For example, this complete version-2 input creates and remounts a file:

```json
{
  "version": 2,
  "volume": {"block_size": 4096, "blocks": 256, "region_size": 64,
             "log_slots": 8, "tree_cache_pages": 4},
  "operations": [
    {"op": "create", "label": "f", "parent": "root", "name": "file", "data": "0042"},
    {"op": "sync"},
    {"op": "remount"}
  ],
  "expected": [{"path": ["file"], "kind": "file", "data": "0042"}]
}
```

The enclosing bundle manifest, run metadata, block trace and semantic flight
record retain their respective version-1 formats. Cache selection is bound
through the preserved scenario and observation. Replay rejects a missing,
mismatched or downgraded observation before semantic execution, including a
resealed bundle whose contents pass its manifest hashes. Version-1 scenarios
retain their original wire encoding and default policy; historical bundles
require their exact observed source and runner identity.

Minimization preserves the volume configuration and includes the profile in the
version-2 scenario's failure signature. An otherwise identical failure under a
different cache configuration cannot replace the original reproducer. Selected
crash outcomes use the same policy for recovered observation.

Run `cargo test -p afsplus-check --test scenario`,
`python3 tools/test-replay-scenario.py` and `python3 tools/test-afsptest.py`.
The Rust ladder covers mkdir/create/write/truncate/rename/unlink/rmdir/sync/remount
under all four profiles and checks exact bytes with both checker views. A
120-file wide-name scenario must change actual block/flush ordering under two
pages on both sides of remount relative to unlimited execution. Python gates
publish and replay the full ladder in fresh processes, check untouched original
artifacts, reject resource/version mismatches, preserve profiles during failure
reduction and verify selected crash outcomes under all four profiles. Wider
mutation-family matrices and their failure artifacts retain their own gates.


## Comparing a rebuilt runner

A rebuilt executable can reproduce every filesystem artifact without sharing the
original executable digest. Exact `replay` continues to require both source and
runner identity. Use a separate comparison for a caller-selected rebuilt runner:

```sh
python3 tools/afsptest.py --runner /private/build/debug/afsplus-scenario \
  compare-rebuilt /private/evidence/original /private/evidence/comparison \
  --source-root /private/reconstructed-source
```

The selected checkout must match the original observed revision and complete
working-tree digest, including unignored files, modes and symlink targets. The
command admits the original metadata, expected-state binding, cache/fault policy,
trace and images before execution. It checks the selected source and executable
identities before and after execution. Neither checkout paths nor executables
are selected by bundle metadata. This command does not perform a build or attest
that the selected executable came from the selected sources.

The comparison requires byte equality of all eight non-metadata roles, including
both images, the full block trace, semantic flight events, scenario, fault model
and expected/actual JSON. Equal final file contents alone are insufficient.
`run.json` records are retained separately: the original reduction history and
executable identity are never transferred to the new run as invented provenance.
Each outcome must agree with its own observation and expected state.

A fresh private output directory contains complete `original/` and `rebuilt/`
bundles and a version-1 `report.json`. The report names both executable digests,
the shared observed source identity, both outcomes, every role's pair of digests,
all differing non-metadata roles and explicit `build_provenance_attested: false`.
It is a comparison record, not an exact-replay certificate. Each nested bundle
has the ordinary integrity manifest; the report is published after both bundles
and its contents are synchronized. A late directory-barrier failure returns an
error even if a complete report can be read. Partial output is retained for
diagnosis and cannot be overwritten by retrying the same destination.

Output inside the selected source tree or original bundle is refused. Original
artifacts are only read; the report preserves copies in separate directories.
The existing per-file and aggregate admission limits apply independently to each
bundle. Retaining two complete copies requires up to twice the aggregate artifact
budget, plus manifests and the comparison report.

Exit 0 means all compared artifacts agree and the rebuilt semantic run passes.
Exit 2 means a semantic failure or artifact difference, including an exactly
reproduced failure. Exit 1 means admission, execution or publication failed.
Inspect both `semantic_artifacts_equal` and the two outcome fields; equality does
not turn a failing scenario into a successful qualification.

Run `python3 tools/test-rebuilt-comparison.py` for all four cache profiles,
distinct executable identities with exact-replay refusal, operation and expected-
state failures, a selected crash, diagnostic drift despite a passing semantic
verdict, metadata/source refusal before execution, source changes at execution,
original preservation, output collisions and publication-failure boundaries.
[Working-source preservation](#preserving-and-restoring-working-sources),
toolchain/dependency preservation, build reconstruction and cross-host qualification
need their own evidence beyond this comparison.


## Preserving and restoring working sources

[replay-source.py](../tools/replay-source.py) captures the Git revision and its
reachable history, index entries and working files needed to reproduce the
observed source identity. The package is private qualification evidence, including
uncommitted and untracked source contents and committed history. It is stored
outside the source checkout and is not published by the tool.

```sh
python3 tools/replay-source.py capture /private/source /private/evidence/source-package
python3 tools/replay-source.py restore /private/evidence/source-package /private/restored-source
```

Capture preserves tracked and unignored paths from the same Git inventory as the
semantic harness. It records regular-file bytes and modes, safe relative symlink
targets and modes, and missing tracked files. The Git index retains staged bytes
independently of working bytes, including all three conflict stages; staged blobs
are preserved even when the HEAD history does not contain them. A second source
scan and revision/index comparison reject changes during capture.

The version-1 `git-working-source-v1` manifest binds the observed revision and
working-tree digest, raw path bytes encoded as hexadecimal, file kinds/modes,
content-addressed blobs, the Git bundle and index objects. Paths are sorted for
the harness's source-digest calculation. Each blob has a size and SHA-256 digest;
Git blob identities are checked independently before restoration. The completion
manifest is published after artifact synchronization, followed by directory and
parent barriers. Existing and partial output paths cannot be overwritten. A late
barrier failure is reported even if the manifest is readable.

Admission limits are 32,768 worktree paths, 4,096 bytes per path or symlink target,
64 MiB per blob, 256 MiB of unique package blobs and independently 256 MiB of
expanded working-file contents. The manifest and Git index/list outputs have an
8 MiB limit. Each Git subprocess has a 120-second deadline, bounded output and
64 KiB of diagnostic output. These are artifact and working-tree bounds, not
measurements of Git's internal pack expansion or peak RAM.

Restoration admits the entire package before creating a new directory. Git
clones a captured local bundle without checkout or templates; the tool writes
admitted files directly and restores the index without executing source scripts,
filters or hooks. It does not copy repository configuration. Global Git
configuration is excluded from restoration commands, and global ignore rules
cannot hide captured untracked paths from the restored inventory. Final source
and index identities must match before the command reports success. The original
package is only read. A restored working directory is a verified build input;
the synchronized package is its retained reconstruction source.

Refuse absolute or parent-traversing paths, Git-internal paths, duplicate paths,
file ancestors, escaping symlinks, unsupported special files/mode bits and Git
submodule index entries. Unsupported filename or symlink semantics on the
receiving host fail explicitly. The package preserves source identity, not all
local repository state: other branch/tag refs, reflogs, ignored build outputs,
repository configuration and index optimization flags are outside this profile.
The HEAD-reachable history and staged objects are sufficient for this profile's
restored Git revision and index; it is not a general repository backup.

Build into an external target directory using the retained lockfile, then use
[the rebuilt comparison](#comparing-a-rebuilt-runner) with the restored source root.
An offline build that uses an existing host cache establishes source restoration
and semantic reconstruction on that host. A self-contained build package also
requires preserved dependencies, toolchain and explicit build-environment inputs;
source restoration alone does not attest these or establish cross-host results.

Run `python3 tools/test-replay-source.py` for dirty/staged/conflicted index
round-trips, missing and untracked files, permissions, Unicode/control-character
names, symlinks, original preservation, path and artifact refusal, independent
expanded-size limits, capture races and publication failure boundaries.


## Retaining registry dependencies for a cold build

[replay-dependencies.py](../tools/replay-dependencies.py) captures the locked
crates.io dependency tree for a selected source checkout using caller-selected
Cargo, rustc and Cargo-cache paths. Capture always invokes `cargo vendor --locked
--offline`; missing cached packages cause a refusal, not an implicit download or
lockfile update. Acquire exact public locked packages separately when preparing
the cache. Local path dependencies, including the patched fuser tree, belong to
the [source package](#preserving-and-restoring-working-sources).

```sh
python3 tools/replay-dependencies.py capture /private/restored-source \
  /private/evidence/dependencies \
  --cargo /private/toolchain/bin/cargo \
  --rustc /private/toolchain/bin/rustc \
  --cargo-home /private/prepared-cargo-cache
python3 tools/replay-dependencies.py verify /private/evidence/dependencies \
  --source-root /private/restored-source
```

The version-1 `cargo-registry-dependencies-v1` manifest binds the complete observed
source identity, `Cargo.lock` digest, observed Cargo/rustc executable digests and
sorted relative paths, sizes, modes and SHA-256 hashes of all vendored files.
Each crate requires its Cargo manifest and registry checksum map. The profile
accepts only the ordinary crates.io replacement emitted by Cargo; additional
registries and Git source replacements require an explicit profile extension.
Verification establishes retained-file integrity and source binding. Cargo checks
registry checksums against the lockfile when resolving and building the retained
sources.

The package contains `vendor/` and a completion manifest. Files and directories
are synchronized before completion publication; source/compiler identity and
vendor inventory changes during capture are refused. Late barrier failure is an
error even if the manifest is readable. Existing or partial output is never
overwritten. Verification only reads the source checkout and package, refusing
missing, extra, modified or mode-changed files, symlinks and special files. Package
paths are relative; a copied package can be verified at another location.

Admission covers 32,768 files and directories each, 64 MiB per regular file,
256 MiB of expanded file data and an 8 MiB manifest. Vendoring has a 120-second
command deadline and 64 KiB limits on both configuration output and diagnostics.
The file limits are checked on Cargo's generated output before manifest
publication; they are not a live quota on Cargo's internal memory or temporary
copying work. The source tree and prepared Cargo cache cannot contain the output.

For a cold build, start with an empty, separately created `CARGO_HOME` and a fresh
external target directory. Use the selected toolchain and explicit environment
inputs. Resolve crates.io exclusively through the retained directory with these
Cargo options, then build the runner with `--frozen --offline`:

```text
--config 'source.crates-io.replace-with="afsplus-retained"'
--config 'source.afsplus-retained.directory="/private/evidence/dependencies/vendor"'
build --frozen --offline -p afsplus-check --bin afsplus-scenario
```

[`cargo_config()`](../tools/replay-dependencies.py) produces this fixed argument
list for the selected package path; package metadata cannot supply commands or
URLs. Verify the package before and after the build. Require an independent
negative control with another empty Cargo home, fresh target directory and an
empty replacement source: it must fail because a locked dependency is absent.
Then run the [rebuilt comparison](#comparing-a-rebuilt-runner) for every retained
cache profile and preserve both outcome and artifact-equality evidence.

Run `python3 tools/test-replay-dependencies.py` for fresh-process capture/verify,
relocation, source/lock binding, file and metadata admission, capture changes,
partial-output refusal and publication-failure boundaries. These protocol tests
use a fixed fake vendor command; the cold build and empty-source negative control
are separate real-Cargo integration evidence. Run `python3 tools/test-replay-source.py`
for the shared bounded-process helper and source-package regressions.

A cold Cargo cache removes dependency-cache reuse from the build evidence. It does
not preserve or qualify the selected compiler's sysroot, linker, platform SDK or
host libraries. Their bytes and build-environment inputs require a separate
retained toolchain/platform profile before claiming self-contained reconstruction.


## Copied host toolchain and SDK qualification

A host reconstruction profile identifies the compiler, sysroot, linker, SDK and
host-runtime requirements separately. For the Darwin ARM64 profile, preserve the
Rust toolchain tree, the selected Clang driver and linker, their non-system shared
libraries, Clang resource directory and selected SDK tree. Preserve regular-file
bytes/modes and relative symlink targets/modes; compare each copy with its source
and retain a content inventory. System libraries referenced under `/usr/lib` and
`/System/Library` are prerequisites of the named macOS host profile, not files
implicitly supplied by the dependency package.

The retained Darwin ARM64 directory uses these roles:

| Role | Relative location | Selection during build |
|---|---|---|
| Cargo and rustc | `rust/bin/cargo`, `rust/bin/rustc` | Explicit executable paths and `RUSTC` |
| Rust sysroot | `rust/` | `--sysroot` in encoded Rust flags |
| Clang driver | `apple/bin/clang` | `-C linker=...` |
| Apple linker | `apple/bin/ld` | `-C link-arg=--ld-path=...` |
| Linker libraries | `apple/lib/` | Preserved relative runtime-library layout |
| Clang resources | `clang-resource/` | Explicit driver `-resource-dir` arguments |
| SDK | `sdk/` | Explicit driver `-isysroot` arguments and `SDKROOT` |

Pass Rust arguments with `CARGO_ENCODED_RUSTFLAGS` so paths are individual
arguments, including spaces. Apply the selected linker and sysroot to host build
scripts as well as target crates. A native host build without an explicit Cargo
`--target` uses this flag scope; inspect verbose compiler invocations rather than
assuming build scripts inherit the intended tools. Record the deployment target,
selected environment, Cargo command and original source/dependency manifest hashes.
An initially empty Cargo home and fresh target directory are independent of the
retained toolchain trees.

Require a frozen offline build and these independent negative controls, each with
its own fresh Cargo home and target directory:

- substitute a nonexistent selected linker; compilation must fail because that
  linker is absent, rather than silently using the installed linker;
- substitute an empty SDK directory in both `SDKROOT` and the explicit linker
  arguments; the build must fail for missing SDK link inputs;
- preserve the [empty registry-source control](#retaining-registry-dependencies-for-a-cold-build).

Then compare all retained cache-profile bundles using the rebuilt runner. Require
exact equality of all non-metadata artifacts and unchanged originals. The copied
file inventory, build inputs, positive/negative logs and comparison reports are
separate evidence. The copy manifest records observations. Use the
[automated reconstruction commands](#automated-host-reconstruction) to seal and
verify the retained directory and execute the build/comparison gates.

A successful documented host-profile reconstruction is distinct from bit-identical
executables, another host's runtime compatibility and physical-device durability.
Qualify other hosts under the [portability stage](../ROADMAP.md#stage-d-portability-and-host-tooling)
and M01/M12 with their own toolchain/runtime profiles. Keep that requirement visible
without making every host platform a prerequisite for Stage A's executable host
core. The complete queue retains the native-provider and sustained-qualification
gates independently.


## Automated host reconstruction

[rebuild-replay.py](../tools/rebuild-replay.py) implements the Darwin ARM64
retained-host profile. Prepare private copies of the four directory roots listed
above; the tool does not copy installed tools or modify their installation.
`seal-toolchain` inventories the copies, validates paths and required roles,
synchronizes regular files and directory entries, rechecks the inventory and host,
and publishes `copy-manifest.json` exclusively. Existing and incomplete manifests
cannot be overwritten. Sealing records the selected bytes, not a successful build
or upstream authenticity.

```sh
python3 tools/rebuild-replay.py seal-toolchain /private/retained-tools
python3 tools/rebuild-replay.py verify-toolchain /private/retained-tools
python3 tools/rebuild-replay.py build \
  /private/evidence/source-package /private/evidence/dependencies \
  /private/retained-tools /private/new-reconstruction \
  --bundle /private/evidence/cache-2 \
  --bundle /private/evidence/cache-4 \
  --bundle /private/evidence/cache-8 \
  --bundle /private/evidence/cache-unlimited
```

Verification checks the manifest schema, required executable/library roles, exact
path inventory, file kinds/modes/sizes/hashes and safe component-relative symlinks.
It streams regular-file hashing rather than loading the SDK into memory. The
profile admits 100,000 file/link entries and directories, 1 GiB per regular file,
4 GiB of regular-file contents, 4,096-byte paths/link targets and a 32 MiB manifest.
Extra metadata/logs at the enclosing directory level are outside the four retained
roots; extra files inside those roots cause refusal. These bounds cover admitted
inputs and do not claim a whole-process RAM or build-output quota.

The recorded Darwin system/release, ARM64 architecture and `sw_vers` values must
match the receiving host. Hostname and kernel description text are observations,
not machine identity restrictions. A different OS profile requires separate
qualification rather than silently bypassing the host check.

`build` admits one to sixteen caller-selected bundles and binds every bundle,
source package and registry package to the same source identity. It verifies tools
before execution, creates a fresh private output directory, preserves all six
reconstruction-driver scripts under `driver/`, restores the source,
verifies dependencies and runs the positive build plus all three negative controls.
Each build uses an independently created empty Cargo home and target directory.
Commands are fixed by the driver; metadata cannot supply shell commands. The
recorded environment contains the selected PATH, Cargo/Rust/linker/SDK settings and
only the existing HOME/TMPDIR values from the calling environment. Other inherited
build flags and wrappers are excluded.

Cargo runs with `--frozen --offline`, explicit Rust sysroot/linker/SDK arguments,
a 300-second command deadline and an 8 MiB combined-output limit. Private logs are
kept on success, command failure or timeout; existing logs cannot be replaced.
Each negative control must return Cargo's diagnostic failure code 101 and contain
the expected absent-linker, missing-System-library or missing-registry-package
finding. An unrelated failure, timeout or unexpectedly successful control cannot
qualify the reconstruction.

The driver compares each original bundle with the rebuilt runner, preserves paired
bundles and rechecks original role/manifest digests. It then revalidates toolchain,
source-package, dependency and driver-source inputs. `qualification.json` binds their manifest
hashes, the host observation, control results, per-bundle cache policy and semantic/
artifact verdicts, plus hashes and sizes of the selected build inputs, logs, rebuilt
executable, comparison reports and retained driver scripts. Python version/executable
identity and Git version are recorded as qualifier-host observations; these host
utilities are runtime prerequisites, not silently included in the copied compiler
tree. The retained scripts can run from outside the development checkout. Evidence
files and directories are synchronized
before publishing this completion record. Partial output is retained; retry uses
a new destination. A failed final barrier reports an error even when a readable
completion record exists.

Exit 0 means the complete selected reconstruction and comparisons passed. Exit 2
means a reproduced semantic failure or artifact difference; failure evidence is
retained. Exit 1 means admission, execution, control qualification or publication
failed. A one-bundle reconstruction is useful for a bug reproducer but does not
establish the complete four-profile cache gate. Require all four explicit profiles
for that acceptance claim. Keep source, dependency, toolchain and original-bundle
inputs alongside the driver and results in retained private storage, such as a
Git-ignored `build/` qualification directory. Copy and verify existing evidence;
never move or overwrite it. Synchronize copied inputs and bind their manifest
hashes to the completed result. Temporary experiments alone are not the retained
input set, and none of these SDK or private-source artifacts is implicitly
published with the repository's code commits.

Run `python3 tools/test-rebuild-replay.py` for sealing and capture changes, host/
path/role/integrity refusal, four-profile orchestration, failure preservation,
negative-control reasons, unchanged originals, input changes, bounded logs and
publication failures. Those protocol tests use fake build tools that copy a runner;
real copied-toolchain builds and their negative controls are separate integration
evidence. Source, dependency and replay component tests retain their own gates.

## Common checkpoint-tail flight recorder

`afsplus_core::flight::FlightRecorder::new(NonZeroUsize)` reserves a caller-sized
ring fallibly. Install it with `Volume::replace_flight_recorder(Some(recorder))`;
passing `None` returns the old recorder and disables emission. Installation does
not change filesystem policy or perform device I/O. Emission uses the reserved
storage, overwrites the oldest record at capacity and counts discarded records.
Sequence and attempt identifiers do not wrap: exhaustion stops recording and
increments the saturating dropped counter. Records are readable in sequence order.

The [common commit tail](../crates/afsplus-core/src/volume.rs) emits begin,
queued-data completion, metadata barrier completion, checkpoint publication begin,
checkpoint barrier completion, and either successful root adoption or failure.
An attempt starts at this tail, not at public API entry. Failed retries may reuse
an on-disk generation but receive distinct recorder attempt IDs. Data completion
covers this tail's queued writes and its conditional barrier; it is not an
assertion about unrelated earlier writes or intent-log durability. A successful
checkpoint barrier followed by a failed root reload emits both facts. Failure
records report whether the volume requires remount because publication may have
started; they do not claim which state survived a real power loss.

The recorder defaults to disabled, performs no clock reads, stores no names or
payloads and adds no on-disk fields. The ring capacity bounds its event storage,
not total filesystem or process memory. [Core API spans](#core-api-call-spans)
provide opt-in call correlation. [Mount and recovery](#mount-and-recovery-observation),
[format](#format-observation) and [verification](#verification-observation)
observation have their own opt-in entry points. Allocator, tree and cache events
have separate integration gates.
Category selection and live adapters follow the contract below. Internal export is provided by
the opt-in [version-3 bundle profile](#internal-diagnostic-bundles); older semantic
profiles retain their original operation-only flight artifact.
[ADR-023](../adr/ADR-023-developer-observability.md) and
[observability](../docs/26-debug-observability.md) govern those integration
requirements.

[Integration tests](../crates/afsplus-core/tests/flight.rs) compare enabled and
disabled device traces and every image block, exercise three-record overwrite,
retry after a metadata barrier fault, checkpoint-write and final-barrier failures,
and failed adoption after a successful barrier. The recorder unit test exercises
fixed allocation capacity and identifier exhaustion. These are host observations;
they neither qualify a physical provider nor close all publication-family gates.

## Category selection and live diagnostics

`FlightRecorder::set_categories` changes future admission using `Categories::ALL`,
`Categories::NONE` or a selection assembled with `with(Category)`. Transaction
covers Begin/Adopted, I/O covers DataWritesComplete, checkpoint covers the three
metadata/publication durability observations, and error covers Failed. These
categories describe the common commit tail, not all activity in those subsystems.

Selection happens after sequence/attempt identity assignment. A filtered Begin
advances the attempt, so subsequent failure events retain their identity. Mask
changes preserve existing records; drains remove them. Both preserve identities
and cumulative counts. `filtered()`
counts intentional exclusions; `dropped()` counts overwrites and identity
exhaustion. Exhaustion stops identifiable events before live delivery. All
counters saturate, and no identifier wraps.

`replace_sink(Some(Box<dyn LiveSink>))` attaches a trusted synchronous adapter;
`None` detaches it and returns its ownership. The adapter's `try_event` callback
receives each selected event after local ring insertion. `Accepted` increments
`delivered()`, `Busy` increments `missed()` and permits subsequent attempts, and
`Closed` increments `missed()` and disables further callbacks. Further selected
events while closed also count as missed. Replacement clears Closed state while
preserving cumulative counters and ring contents; no adapter is dropped inside
emission. Detached or deliberately filtered events do not count as missed.

The callback runs inside filesystem operations and must be bounded, nonblocking,
nonallocating, non-reentrant and non-unwinding. It must not mutate filesystem
state. A preallocated queue moves arbitrary consumer processing outside that
operation. Explicit Busy/Closed results cannot change the filesystem result;
arbitrary host-code panics or execution time are outside this trusted-adapter
contract. This is an optional Rust diagnostic interface, with no filesystem API
v2 ABI, capability or disk-format change. Uninstalled diagnostics allocate no ring
or adapter; small ring/queue capacities are valid with observable loss.

The recorder owns only the caller-bounded ring, fixed counters and an optional
boxed adapter. Adapter state and any transport queue have separate caller-owned
bounds. Layout qualification and the optional API scope are described under
[core API spans](#core-api-call-spans). Tests use a one-event bounded channel to exercise delivery, saturation,
consumer disconnection and reattachment without requiring a live network or GUI.
The ring's overwrite count and the live channel's missed count are independent.

The [core tests](../crates/afsplus-core/tests/flight.rs) compare every block,
device trace and operation result across 16 selections, 2/4/8/unlimited cache
profiles and successful/failed barrier paths. A bounded consumer has separate
four-profile failure checks. Unit cases cover mask changes inside an attempt,
drain, exhaustion, Busy/Closed behaviour, replacement and Send/Sync preservation.
These gates do not establish arbitrary adapter safety or other subsystem coverage.

Semantic profiles through version 3 use ALL categories and no live adapter.
Their AFSPSC03/AFSFLT02 bytes and prior replay identities are unchanged.
[Version 4](#selected-category-and-live-delivery-bundles) binds selected categories
and deterministic live-delivery observations without interpreting intentional
gaps as overwrites.

## Core API call spans

Call `FlightRecorder::enable_api_observation()` before installation to observe
mutable operational `Volume` entry points, including nested calls and refusals
before commit. The default scope emits only the common commit-tail events;
semantic profiles 1 through 4 preserve that scope and their original wire bytes.
[Version 5](#api-and-window-replay-bundles) exports API and window context.
Legacy export profiles reject extended kinds rather than dropping them.

Each observed call receives a monotonic span ID, its parent's span ID and the
root call's operation ID. Top-level calls have parent zero and use their own
span as the operation ID. `ApiMethod` assigns explicit append-only diagnostic
IDs; these have no filesystem API v2 ABI meaning. Commit events carry the active
API context, their existing commit-attempt ID and checkpoint generation. API
events use attempt zero. One root call can contain several commit attempts.
Calls which stage work into an operation window and a subsequent commit call
have separate roots; the window identity below joins their deferred work.

The guard emits `ApiBegin` before validation and `ApiSucceeded` or `ApiFailed`
on return. During Rust unwinding it emits `ApiUnwound` and restores the parent
context. Process abort cannot run that guard. Success describes a returned
result; durability follows the operation's contract and independent recovery
checks. Failure or unwinding does not imply rollback. The remount flag samples
the volume's known remount-required state at each event, including unsafe
window mutation or publication failures.

`Category::Api` selects admission after identity assignment, so filtered API
events leave detectable sequence gaps and commit events retain their context.
The runtime all-category mask includes API bit 4, window bit 5 and object bit 6;
version-4 bundles accept only the original four category bits, and version 5
accepts bits 0–5. Object observation requires its separate runtime opt-in. Span exhaustion stops emission with saturating loss
accounting, preventing identity reuse or attribution to a stale parent.
A fresh recorder starts a separate identity domain; callers must retain
that boundary when combining recordings. Reattaching the same recorder preserves
its counters but starts a new observation of an existing window.

Storage is caller-bounded. Ring construction reserves event storage; installing
it on a volume separately allocates a shared recorder owner. Emission reads no
clock and stores no file names or payloads. Requested ring storage is
`capacity * size_of::<Event>()`, plus allocator rounding. The earlier macOS
AArch64 probe (104-byte events, 176-byte recorder) predates subsystem payloads
and shared ownership; those numbers must not be used for the extended recorder.
Account separately for the shared owner, synchronization and reference counters,
transaction observer handles, optional adapter state and transport storage.
A volume without a recorder allocates no ring or shared recorder owner. Small
capacities preserve filesystem semantics while exposing overwritten or missed
diagnostics. Every target layout and native resource budget needs measurement.

Recorder access returns a read-only guard; multiple readers may coexist.
Configuration and emission require an exclusive write guard. Drop guards before
conflicting access, which fails immediately without waiting. Transaction
observers hold weak references and cannot retain a detached recorder. Replacing
a recorder updates observers in an open operation window. These Rust diagnostic
handles do not change filesystem API v2, capability decisions or disk records.

[Core tests](../crates/afsplus-core/tests/flight.rs) compare observed and plain
operation results, full device traces and every image block at 2/4/8/unlimited
cache sizes. Cases cover all three non-empty-file barriers, nested calls,
duplicate-name refusals, snapshot preservation, protection changes, pinned-view
deletion refusal and a provider unwind before writes. Unit tests cover filtering,
exhaustion and context restoration. The
[coverage guard](../crates/afsplus-core/tests/api_coverage.rs) checks registered
mutable entries in the three Volume implementation sources against their outer
wrappers. Pure getters, raw-device access, recorder control, constructors,
mount/recovery, handle destruction and platform adapters need their own scope.
[Object-map observation](#object-map-observation) supplies record-block/view linkage;
other block roles, subsystem events and platform coverage retain the
[internal coverage queue](../implementation/audit-work-queue.md#complete-work-queue).

### Publication-family observation equivalence

Run the `afsplus-core` integration test
`publication_families_preserve_results_images_and_io_with_small_rings` in
[flight.rs](../crates/afsplus-core/tests/flight.rs). It exercises directory
creation, symlink creation, hardlink creation, rename, whole-file clone, unlink,
truncate and protection updates independently from the same formatted fixture.
Each family runs at 2/4/8/unlimited cache pages, with no injected failure or a
failure at flush index 0 or 1, and with ring capacity 1 or 1024.

For each combination, compare the exact result, block-I/O trace and every image
block between unobserved and observed execution. Require a successful result
without injection and an error with injection. The retained final API event
must identify the outer method and its matching outcome. A one-event ring must
report loss; the larger ring must retain a commit-begin event without loss.

These are observation-equivalence tests, not power-cut or mount-recovery
qualification. They cover the two selected flush indices, not every possible
write, read or barrier failure. Internal object/block/view event coverage has
its separate owner in the
[diagnostic inventory](../implementation/milestones.md#core-diagnostic-path-inventory).

## Object-map observation

`FlightRecorder::enable_object_observation` enables object resolution and API
identities. `Category::Object` (runtime bit 6) controls event admission; filtering
API records does not disable their identity context. Default recorders do not
emit object events, preserving the sequence and event vocabulary of replay
profiles through version 5. Those profiles neither export object payloads nor
accept the new category bit. [Version 6](#object-map-replay-bundles) defines
the object export contract.

`ObjectLookup` identifies the requested object before object-map I/O.
`ObjectMapped` identifies the resolved metadata block; it does not assert that
subsequent reads, checksums or metadata validation succeeded. `ObjectMissing`
means a successful lookup returned no entry. A lookup I/O error leaves its
attempt visible and terminates the enclosing API with failure rather than
reporting a missing object. A live root-cache hit identifies the cached record
block without issuing an extra disk read.

The event's optional object context contains object ID, metadata-block address
and view ID. View zero denotes the live committed map; a positive view ID is
the persistent snapshot ID. Event generation denotes the map's viewed
checkpoint, not the metadata block's birth generation. Attempt/missing records
use block zero; mapped records contain the returned block. Object events have
commit-attempt zero, since resolution can precede publication. API/root/parent
and window context provide operation correlation. The object payload belongs
only to its resolution event and cannot carry over into a sibling or commit
record. A recording must retain its volume identity externally; identifiers
are not globally unique across volumes or independent recorders.

Captured stat, allocation enumeration, link reads, file reads, lookup and
directory enumeration borrow an explicit observer after validating the handle.
They use the captured object-map root and generation. The observer neither
allocates storage nor performs a lookup to enrich its event; it uses values
already obtained by the filesystem. Recorder storage and live-adapter limits
follow the [API observation bounds](#core-api-call-spans). A constrained host
can omit the recorder or select a smaller capacity; loss remains explicit and
filesystem behavior must be unchanged. Measure each native ABI separately.

Run [flight tests](../crates/afsplus-core/tests/flight.rs):

- `object_resolution_links_live_and_captured_blocks_without_extra_io`: all four
  cache profiles, one/2048-event capacities, exact live/captured bytes, six
  captured API identities, root-cache hits, missing objects and distinct
  live/captured record blocks, with whole-image and I/O equality.
- `object_observation_preserves_publication_family_failures_and_images`: the
  192 [publication-family pairs](#publication-family-observation-equivalence),
  retaining returned IDs as well as errors, with object observation enabled.
- `failed_object_lookup_is_not_reported_as_missing_and_retry_keeps_its_identity`:
  first post-mount lookup read failure, exact error/I/O/image equality, no false
  missing/mapped result, successful retry under a new API operation identity.

The recorder unit test `object_filtering_loss_and_scope_do_not_reuse_commit_identity`
checks opt-in, filtering, one-record loss, commit-attempt separation and absence
of payload inheritance. `object_payload_reaches_bounded_live_delivery_with_explicit_loss`
checks the object payload through a one-event live queue, saturation and consumer
disconnection. Allocation/tree/cache/reclaim transitions, data-block
roles, mount-time recording and extended export retain their explicit tracker
requirements; resolving a metadata address does not satisfy them.

## Deferred-window observation

API observation also assigns a recorder-local monotonic `window` identity to
work staged across API calls. Zero means outside an observed window. Window
lifecycle events use attempt zero and the window's target generation; checkpoint
events retain their commit attempt and active window context. An empty window
can close without changing the checkpoint generation, so generation alone is
not a window identity.

`WindowOpened` observes creation; `WindowAttached` observes an existing window
when recording begins. `WindowLogBegin` and `WindowLogFailed` identify the
attempted intent-group sequence. `WindowLogDurable` acknowledges that sequence;
other events carry the last observed durable group, or zero. `WindowFailed`
reports a failed mutation or pre-record barrier with that acknowledged sequence.
The remount flag distinguishes poisoned state from ordinary validation refusal.
`WindowClosed` ends the context; it does not by itself prove publication,
rollback or recovery. Snapshot registry publication after a window checkpoint
has window zero and its own commit attempt.

Drain through `Volume::flight_recorder_mut()` to preserve context across calls.
Removing a recorder emits `WindowDetached`; reattaching it observes a new window
identity, because intervening work is unknown. Enabling observation through the
mutable accessor attaches before the next operational API event. Filtering
retains context for admitted events. Identity exhaustion stops emission with
loss accounting and never reuses an identity. These window hooks cover staged
work inside a mounted volume; the mount path has its own entry points under
[mount and recovery observation](#mount-and-recovery-observation), and process
abort and handle destruction have separate owners.

The [flight tests](../crates/afsplus-core/tests/flight.rs) compare results, full
block traces and complete images against unobserved execution under all four
cache profiles. They exercise deferred groups, validation refusals, data-write
and barrier failures, snapshot publication, draining, reattachment, empty windows
and late enablement. Unit tests cover filtering and identity exhaustion. Legacy
profiles 1–4 leave API/window observation disabled; their encoder rejects these
extended event kinds; [version 5](#api-and-window-replay-bundles) defines their fields.

## Mount and recovery observation

`afsplus_core::mount_observed(device, options, recorder)` and
`mount_observed_with_snapshot_limits(device, options, limits, recorder)` attach a
caller-owned recorder before checkpoint selection. A mount which returns a volume
moves the recorder into it with sequence, attempt and cumulative counters intact,
so `Volume::flight_recorder()` and `Volume::replace_flight_recorder(None)` read
the mount observations together with every later operation. A refused mount
returns `Box<RefusedMount>`, holding the `CoreError` the unobserved entry points
report and the recorder with the stages which ran. `mount`, `mount_with_options`
and `mount_with_snapshot_limits` keep their signatures and their unobserved path.

`MountBegin` opens the observation at generation zero, before identification I/O.
`MountSelected` carries the chosen generation, its slot (0 for A, 1 for B) and the
other structurally valid checkpoint's generation, or zero. `MountIntentBegin`
reports the intent-log slot count, `MountIntentScanned` the valid prefix record
total with that prefix's last group sequence and a damaged-tail flag,
`MountIntentReplayed` one group's operation count at its sequence, and
`MountComplete` the replayed (writable modes) or pending (read-only modes) record
count at the volume's final generation. `MountFailed` names the stage which
refused: identification, feature negotiation, checkpoint selection, bounded root
state, budget configuration or intent log. Every mount event carries the mount
mode and uses attempt zero; replay publication emits the ordinary commit-tail
events with their own commit attempt. Scan events carry the committed generation
the records bind to, and replay events the generation the recovery batch
publishes.

Attachment changes no mount semantics, reads no additional block and writes none:
observed and unobserved mounts of one image produce the same result, the same
block trace and the same image. Mount keeps its bounded root loading, so
exhaustive checking has its own entry points under
[verification observation](#verification-observation). Cache and snapshot budgets
applied during mount carry no API span, because the recorder enters the volume
after those budgets are set. Category selection, live delivery, ring loss and
identity exhaustion follow the
[common contract](#category-selection-and-live-diagnostics). Host observation is
the scope here; the native handler, the FUSE and FSKit adapters and physical
media have separate owners.

The [flight tests](../crates/afsplus-core/tests/flight.rs) compare an observed
mount with an unobserved one at 2/4/8/unlimited tree-cache pages over six images:
a clean volume, a volume with two durable intent groups replayed, the same volume
inspected under NO_CHANGES, a volume whose second log slot holds an unreadable
nonzero record, an ambiguous volume whose slots carry one generation, and a
persistent-snapshot volume refused by feature negotiation. Each case compares the
mount result, the full block trace and every image block, and asserts the exact
event sequence, stage, group sequences and counts. Ring capacities 1, 2 and 4
report every overwritten record with identities equal to those of an unbounded
ring, and a mount-only category mask reproduces exactly the mount events of an
unfiltered run with the intentional exclusions counted as filtered. Recorder unit
tests cover identity exhaustion, which stops emission and counts the refusal.

## Format observation

`afsplus_core::mkfs_observed(device, params, options, recorder)` formats with a
borrowed recorder, because a formatter owns no volume. `mkfs` and
`mkfs_with_options` keep their signatures and their unobserved path.

`FormatBegin` opens the observation at the device block count read on entry.
`FormatMetadataDurable` reports the barrier after the metadata, identification
and slot-B zeroing writes. `FormatPublicationBegin` precedes the slot-A
checkpoint write and carries that slot address, and `FormatCheckpointDurable`
reports the final barrier. `FormatFailed` names the stage which failed:
parameter validation, metadata writes, the metadata barrier, publication or the
publication barrier. Every format event carries the generation a formatter
publishes and uses attempt zero.

Formatting keeps its documented non-atomic contract: an interrupted format
leaves a device with no valid AFS+ volume, and observation preserves that
outcome byte for byte. The [flight tests](../crates/afsplus-core/tests/flight.rs)
inject a fault at each write and at each of the two barriers of a 256-block
format, comparing observed and unobserved runs for the returned error, the full
block trace, every image block and the outcome of mounting what survived. The
successful case asserts the four-event sequence and its slot addresses, each
failure asserts the failing stage and that no checkpoint durability is claimed.
A one-event ring reports the three overwritten records, and a mask without the
format category records nothing while counting four intentional exclusions and
producing a mountable image.

## Verification observation

`afsplus_core::verify::load_mount_state_observed`,
`load_committed_state_observed` and `full_sweep_observed` take a borrowed
recorder alongside the arguments of `load_mount_state`, `load_committed_state`
and `full_sweep`, which keep their signatures and their unobserved path. Normal
mount uses the unobserved bounded loader, so exhaustive verification is a
separate caller decision.

`VerifyBegin` opens one verification, carrying its scope: bounded mount state,
committed state or full sweep. `VerifyPhase` names the structure being decoded
or swept, with its root block where the checkpoint holds one: allocation root,
shared extents, object map, object records, shared mappings, namespace, reclaim
queue, allocation bitmaps, intent-log area, snapshots, root object, root
directory, link counts, quarantined runs, bitmap accounting and free counts.
`VerifyFinding` reports one full-sweep finding at its ordinal in the returned
list, with the finding class and the object, block or region the finding names.
`VerifyComplete` closes a verification and carries the finding count of a full
sweep. `VerifyFailed` reports the phase which raised an error. Verification
events use attempt zero and the verified checkpoint's generation.

Observation adds no device read and no allocation, so an observed verification
reads the same blocks and returns the same findings in the same order as an
unobserved one. The [flight tests](../crates/afsplus-core/tests/flight.rs) run
all three entry points over a clean image, an image whose checksum-valid object
record contradicts its namespace link count, and an image with an unreadable
object-map root, comparing loaded state, findings and the full block trace with
unobserved execution. They assert the phase sequence of each scope, the failing
phase of the refused loads, and one event per finding at its ordinal naming the
same object the finding text names. Small rings report every overwritten record
with identities equal to those of an unbounded ring while returning identical
findings. These are host observations over deterministic images; physical media
and adapter integration have separate gates.

## Pre-tail and read-path observation

`FlightRecorder::enable_data_observation()` observes file data staged before
the common commit tail, and `enable_view_observation()` observes read-only tree
descents. Both imply the [core API scope](#core-api-call-spans), so their events
carry the enclosing call and window identities, and both default to disabled.

A window create writing its content through, an existing-file write and the
partial tail block a truncate zeroes each emit `DataWriteBegin` with the object,
the logical byte range of the request and the physical run holding it, one
`DataWriteComplete` per physical run with that run's block-aligned range, and
`DataWriteFailed` naming the single block a failed write reached. The scope
field separates the three sites. Block counts saturate at `u32::MAX`, matching
the on-disk extent bound. An fsync owns two further I/O events: `IntentDataDurable`
for the barrier a record's existing-file updates require before log publication,
and `IntentEmptyFlush` for the flush of a group with nothing to log. Both are
distinct from the commit tail's `DataWritesComplete`, which covers the queued
writes of one publication.

Lookup, enumeration and file reads emit `ViewReadBegin` and either
`ViewReadComplete` or `ViewReadFailed`, carrying the read path, the view
identity (zero for the live committed view, a snapshot ID for a captured view),
the owning object and the tree root block. A file-data descent resolves its own
root, so it names the object and leaves the root block zero. Snapshot creation,
deletion and ledger maintenance report `ViewMaintenanceFailed` with the view the
operation names; a maintenance pass covers every view, so it carries the ledger
scan position it reached. Object-map resolution keeps its separate
[object events](#object-map-observation).

Events use attempt zero, are admitted by the Data, View and I/O categories,
follow the common filtering, loss and live-delivery contract, and add no device
I/O and no allocation.

The [flight tests](../crates/afsplus-core/tests/flight.rs) run a window create,
an existing-file write, a truncate, a group needing the data barrier and an
empty group at 2/4/8/unlimited cache profiles, with and without a fault on a
staged write, comparing results, the full block trace and every image block with
unobserved execution, then assert the event sequence, scopes, byte ranges and
physical runs. The read-path tests do the same for four live-view calls and four
captured-view calls, assert the view identity, owner and root of each descent,
and require one API span per observed call. Small rings report their loss and a
single-category mask reproduces exactly the selected events.

## Publication-family execution evidence

Each direct publication caller in [Volume](../crates/afsplus-core/src/volume.rs)
has a named test in the [flight tests](../crates/afsplus-core/tests/flight.rs):
`reclaim_step`, `clone_file`, `clone_range`, `create_leaf_in_directory`,
`create_directory`, `cleanup_orphan_data_step`, `ensure_orphan_directory`,
`remove_entry`, `link_file`, `rename_internal`, `commit_staged_file_layout`,
`materialize_batch`, `commit_object_metadata`, `commit_snapshot_change` and
deferred-window publication.

One harness runs each family over 2/4/8/unlimited cache profiles with its normal
case, its validation refusal where it has one, a fault at its first, middle and
last own write, and a fault at each of its own barriers. Preparation runs before
the recorder is attached and before a fault is armed, so a fault meets the family
itself. Every run compares the observed execution with an unobserved one on the
returned results, the full block trace and every image block, and closes with a
one-event ring which loses records while returning the same results.

Retained events are checked by one correlation oracle: API spans nest with a
matching outcome per span, every subsystem, object, staged-write and read-path
event names the innermost open call, commit events bind a nonzero attempt while
every other kind uses attempt zero, and one attempt keeps one generation and one
root operation from its begin through adoption or failure. A fault-free run
reports a checkpoint barrier; an injected fault reports a failure. Cleanup of an
unknown or non-orphan object answers with zero progress, so that family declares
no validation refusal.

## API guard execution

[Source registration](../crates/afsplus-core/tests/api_coverage.rs) proves that
each guarded entry names its own `ApiMethod`. The execution test in the flight
tests runs all 66 registered methods against one volume with persistent
snapshots, shared extents, the data policy and an intent log, then requires each
to appear as `ApiBegin` with its own identity and a matching outcome closing that
span. A registered method which never executes fails the test, and so does an
unwound span. Results are deliberately unchecked: the guard owns the call
whatever the filesystem answers.

## Diagnostic mechanism qualification

Emission keeps the storage reserved at construction across every payload, an
attached live adapter, category filtering and identity exhaustion. Heap
qualification lives in the [measurement crate](../crates/afsplus-measure/src/main.rs),
which already owns a counting allocator, because the filesystem crates forbid
unsafe code: an identical filesystem round with and without an attached recorder
requests the same heap bytes, so an emitted event costs no allocation. Replacing
a recorder inside an open window keeps staged-write and read-path observation and
assigns a fresh window identity. Unwinding, nested API context, saturation and
failure ordering are covered by the API, window and family tests above.

Host cost on a macOS AArch64 host, release build, uncontended:

| Measurement | Unobserved | Attached |
|---|---|---|
| Ring emission of every payload | none | 26.6 ns per event, 37.6 M events/s |
| Ring emission with an accepting adapter | none | 27.2 ns per event, 36.8 M events/s |
| One repeated `lookup_root` call | 29.98 us per call | 31.61 us per call, 8 events per call |
| Sixty rounds of create, read, lookup and list | 1.79 k operations/s | 1.44 k operations/s, 38 events per operation |

Layout on the same host: one event occupies 232 bytes, of which 40 bytes are
the mutually exclusive mount, format, verify, staged-write and read-path
payload; a recorder occupies 176 bytes, its shared cell 192 bytes, and a shared
owner or a weak subsystem observer one pointer each. A ring therefore requests
232 bytes for one event, 58 KiB for 256 and 928 KiB for 4096, reserved once at
construction.

Both cost measurements are `#[ignore]`d and run with
`cargo test --release -- --ignored --nocapture`. A call path costs about 205 ns
per event, of which about 155 ns is spent before category admission: the
recorder is locked and the payload is built at the call site, so a narrow
category mask saves little against a detached recorder. These are host numbers
for one machine; adapters, physical media and other hosts have separate owners.

## API and window replay bundles

Semantic JSON version 5 enables core API/window observation before executing
operations. Its `AFSPSC05` geometry has the version-4 fields, with category mask
0 through 63: bits 0–3 retain their meanings, bit 4 selects API events and bit 5
selects window events. Observation assigns identities before category filtering.
Profiles 1–4 retain their original recording scope and wire bytes.

Version 5 also admits `window_write` and `window_truncate`, with the same label,
range and payload limits as `write` and `truncate`, plus argument-free
`window_fsync` and `window_commit`. These invoke the deferred core APIs on an
existing file; direct mutations retain their existing behavior. Use explicit
fsync, commit and remount steps to state which durability boundary is being
qualified. Ending a scenario is not an implicit commit of staged work.

`AFSFLT04` retains the 28-byte selected-profile header, 29-byte operation records
and 53-byte batch headers from version 4. Each retained event is 64 bytes:

| Field | Encoding |
|---|---|
| Event sequence, commit attempt, generation | Three little-endian u64s |
| Kind, remount-required flag | Two u8s |
| Root operation, API span, parent span | Three little-endian u64s |
| API method | Little-endian u16; zero for no API context |
| Window identity | Little-endian u64; zero outside observation |
| Intent-group sequence | Little-endian u32 |

Kind codes 1–7 retain their commit meanings. Codes 8–11 are `ApiBegin`,
`ApiSucceeded`, `ApiFailed`, `ApiUnwound`. Codes 12–19 are `WindowOpened`,
`WindowAttached`, `WindowLogBegin`, `WindowLogDurable`, `WindowLogFailed`,
`WindowFailed`, `WindowClosed`, `WindowDetached`, in that order. Method IDs 1–66
are the explicit `ApiMethod` assignments in the [core recorder](../crates/afsplus-core/src/flight.rs).
An extension of the accepted ID set requires coordinated producer/admission
changes. Wire size is independent of the in-memory Rust event layout.

The [API contract](#core-api-call-spans) and [window contract](#deferred-window-observation)
define the fields' semantics. API/window events have attempt zero; commit events
retain a positive, monotonic commit attempt. A sequence can contain API events
without a commit. Admission preserves version-4 loss and delivery equations,
checks root/parent ordering, method and kind domains, zero-context consistency,
window/group bounds and consistent span context within each retained batch.
Missing events cannot prove a complete call tree or lifecycle; exact replay and
independent image inspection supply separate evidence. The format does not
claim mount/recovery instrumentation or native-platform qualification.

[Replay tests](../tools/test-afsptest.py) exercise category selection and ring
loss on four cache profiles with identical images and block traces, malformed
API fields, minimization, selected cuts and two durable groups linked across
separate API roots. Cuts immediately before and after the first group's durable
boundary require exact old or acknowledged file bytes after remount.

## Object-map replay bundles

Semantic JSON version 6 uses `AFSPSC06`, preserves version-5 commands and
geometry, and admits category masks 0–127. Bit 6 selects object-map events.
Object observation also enables API identities, even when API records are
filtered. Earlier versions retain their original event scope and bytes.

`AFSFLT05` retains the selected-profile, operation and batch headers. Each
89-byte event contains the complete 64-byte API/window event followed by a
one-byte presence flag and three little-endian u64s: object ID, resolved
metadata-block address, and view ID. Kinds 20, 21 and 22 mean `ObjectLookup`,
`ObjectMapped` and `ObjectMissing`; their presence flag is one. Other events
have flag zero and all three object fields zero. Lookup and missing events
have block zero. Object events have commit-attempt zero. Their generation
identifies the viewed checkpoint, not allocation birth.

A mapped address is an observation before range/read/checksum validation;
the wire reader must not mistake it for an integrity verdict. The presence
flag, kind, unused fields, category, API context and loss/delivery accounting
are independently checked. Truncation is refused before unpacking payloads.
Replay compares the exact emitted artifact in addition to filesystem state.
The version-6 command set exercises live-view resolution (view zero).
[Version 7](#captured-snapshot-replay-bundles) adds retained-view operations.

Run the [replay tests](../tools/test-afsptest.py) for four cache profiles,
zero/object/all category masks, one/256-event rings, deterministic consumer
disconnection, exact image/I/O comparisons and malformed object controls.
The [scenario admission tests](../tools/test-replay-scenario.py) enforce
version and category bounds before invoking the runner.

## Captured snapshot replay bundles

Semantic JSON version 7 uses `AFSPSC07` and the version-6 diagnostic policy.
It requires `snapshot_limits` with positive `max_edit_records` (at most 4096),
`max_views` (at most 16), and `reclaim_records` (at most 4096). The three decimal
values follow the version-6 format fields in that order. Formatting explicitly
enables persistent snapshots; initial mount, explicit remount and recovered
inspection all receive the same limits before recovery. These are harness
admission bounds, not filesystem scalability limits.

`snapshot_create`, `snapshot_open`, `snapshot_close`, `snapshot_delete` and
`snapshot_inspect` take a scenario-local `label`. Snapshot labels occupy their
own namespace, remain associated with their persistent ID after deletion and
cannot be reused. A remount closes every runtime handle. Opening again resolves
the persistent ID on the new mount. Deletion with an open reader reports the
core refusal. Invalid handle operations produce captured operation failures,
not silently repaired sequences.

Recovered observation enumerates every registered snapshot independently of
live object labels. It checks lookup against directory enumeration, reads file
contents and opaque symlink targets, and records metadata and allocation ranges
(including unwritten ranges). It traverses nested directories with paginated
reads and refuses cycles, repeated paths, missing objects, incomplete reads,
non-progressing pages and exhausted budgets. Four cache profiles use this same
inspector. Aggregate limits across all views are 1024 entries, 16 MiB of content
and 4096 allocation ranges, with depth at most 64; direct inspector callers may
choose smaller budgets. An inspection failure never becomes an empty registry.

`AFSOBS04` preserves the cache, run, checker and live namespace records of
`AFSOBS03`, then adds a mandatory captured section. `snapshots error HEX` records
failure; `snapshots ok COUNT` precedes that many views. Each view begins with
`snapshot ID GENERATION TRANSACTION ENTRY_COUNT`, then `root METADATA` and its
sorted entries. Each `entry PATH METADATA DATA RANGE_COUNT` is followed by
`range OFFSET LENGTH UNWRITTEN` records. Paths are comma-separated hex UTF-8
components; data and errors are hex, with `-` for empty data. Metadata fields are
object ID, kind, size, allocated bytes, link count, protection, created seconds
and nanoseconds, modified seconds and nanoseconds, changed seconds and
nanoseconds, and content generation. Unwritten is exactly zero or one.
`AFSFLT05` retains its version-6 layout; captured object events carry nonzero
persistent view IDs.

The scenario supplies `expected_snapshots` explicitly, independent of observed
output. Each view contains `id`, `generation`, `committed_tx_id`, `root` and
`entries`; each entry contains `path`, `metadata`, `data` and `allocation`.
Metadata uses `object_id`, `kind`, `size`, `allocated`, `links`, `protection`,
`created`, `modified`, `changed` and `content_generation`; timestamps are
`[seconds, nanoseconds]`. Ranges contain `offset`, `length` and boolean
`unwritten`. Views and paths must be sorted without duplicates. Version-7
`expected.json` binds both `entries` and `snapshots`; earlier versions retain
their original bytes. Success requires exact equality of the complete expected
history and live namespace, successful operation/inspection outcomes and clean
structural checks. Replay and reduction retain these requirements and snapshot
resource policy. Wrong historical metadata, allocation or registry membership
is a failure even when every file's contents match.

`snapshot_inspect` exercises an open handle and validates traversal/read
consistency. The complete semantic oracle applies to the final recovered
registry; it does not claim expected-content comparison of a view deleted
before final inspection. The version-7 command set creates files and
directories; direct inspector tests additionally cover hard-link aliases and
symlinks, which [version 9](#linked-namespace-replay-bundles) creates through
scenario commands. These host replay tests do not qualify a native handler or hardware.

Run [replay tests](../tools/test-afsptest.py),
[admission tests](../tools/test-replay-scenario.py) and
[captured inspector tests](../crates/afsplus-check/tests/captured_scenario.rs).

## Subsystem and lifecycle replay bundles

Semantic JSON version 8 uses `AFSPSC08`. It inherits the version-7 command set,
geometry, cache profile, snapshot limits, persistent-snapshot formatting and
`expected_snapshots` contract, and widens the category mask to 0-32767, one bit
per runtime category 0 through 14. Version 9 belongs to a separate profile and
these parsers refuse its fields at version 8.

Version 8 opts the recorder into subsystem, data and view observation before
the image exists, so the runner formats through `mkfs_observed` and mounts
through `mount_observed`, carries one recorder across every remount, and drains
one explicit pre-mount batch holding the format and the first mount. That batch
precedes the operation records and uses the same counter and event framing.
Every later operation drains its own batch the way version 3 does.

Two commands make the remaining scopes reachable from a memory image. `verify`
runs the bounded root load, the exhaustive committed decode and the invariant
sweep over a second handle on the same image, which reads blocks and writes
none. `remount_refused` mounts the persistent-snapshot feature without its work
budgets, so feature negotiation refuses the attempt before the first device
write and hands the recorder back, and the ordinary remount follows.

`AFSFLT06` retains the selected-profile, operation and batch headers and the
complete 89-byte version-6 event, then appends a fixed 49-byte payload area.
Every integer is little endian. Kind codes 1 through 22 keep their version-6
meaning; codes 23 through 64 follow the declaration order of `EventKind` in
[flight.rs](../crates/afsplus-core/src/flight.rs): 23-26 allocation, 27-31
tree, 32-38 reclaim, 39-45 mount, 46-50 format, 51-55 verify, 56-58 staged
data, 59-60 the two intent-group barriers, 61-64 view descents.

| Offset | Width | Field |
| --- | --- | --- |
| 0 | u8 | payload tag: 0 absent, 1 allocation, 2 tree, 3 reclaim, 4 mount, 5 format, 6 verify, 7 data, 8 view |
| 1 | u8 | mount mode, format stage, verify scope, data scope, view path |
| 2 | u8 | mount stage, verify phase |
| 3 | u8 | mount slot, verify finding class |
| 4 | u8 | mount damaged-tail flag |
| 5 | u32 | mount record count, verify region, data block count |
| 9 | u64 | allocation start, tree owner, reclaim root, mount other generation, format block total, verify ordinal, data object, view ID |
| 17 | u64 | allocation blocks, tree block, reclaim start, format publication block, verify object, data offset, view owner |
| 25 | u64 | tree resident pages, reclaim blocks, verify block, data length, view block |
| 33 | u64 | data physical run start |
| 41 | u64 | reserved |

Every byte a payload class does not name is zero, so a reserved byte carries no
value and absence is expressed through the tag with the per-kind rules below.
The tag equals the tag of the event kind's category, so a lifecycle event
without its payload and a commit-tail event carrying one are both refused.

Admission checks the kind, category and tag domains, then the per-kind rules.
Mount mode is 0-3, stage 1-6, slot 0-1 and the damaged-tail flag 0-1;
`MountBegin` carries the identification stage with generation zero and zero
selection fields, `MountSelected` carries the selection stage, the four
intent-log kinds carry the intent-log stage, the damaged-tail flag belongs to
`MountIntentScanned` alone, and a record count belongs to the four intent-log
kinds alone. Generation zero is admitted for `MountBegin` and `MountFailed`
alone, and a zero generation forces zero slot, count, flag and other
generation. Format stage is 1-5 and equals the stage of its kind except on a
refusal, the device block total is nonzero, and a publication address belongs
to `FormatPublicationBegin` and `FormatCheckpointDurable`. Verify scope is 1-3,
phase 0-16 and finding class 0-11; a phase is present on every kind except
`VerifyBegin` and `VerifyComplete`, a finding class belongs to `VerifyFinding`
alone, an ordinal belongs to `VerifyFinding` and `VerifyComplete`, and the
scope-level kinds name no region, object or block. Data scope is 1-3 and view
path is 1-4, and a view descent names a nonzero owner.

Context and accounting rules follow versions 6 and 7: object presence, API span
parentage, method identity and window identity keep their checks, and loss,
filtering and live-delivery equations follow version 4 across the pre-mount
batch and every operation. The two intent-group barriers belong to the
commit-tail I/O category and carry the transaction attempt; every other
extended kind reports attempt zero. A mount observation names its intent-log
group without a deferred window, which is the one exception to the version-5
window rule. Truncation is refused before any payload is unpacked, and an event
byte changed after recording fails admission or exact replay.

Run the [replay tests](../tools/test-afsptest.py) for four cache profiles,
masks of zero, each scope alone and all bits, one and 256-event rings,
deterministic consumer disconnection, exact image and block-I/O comparisons,
a synthetic wire covering one valid record per payload class against malformed
controls for every new field class, fresh-process replay, a changed event byte
and a reduction that keeps the diagnostic policy in the failure signature. The
[scenario admission tests](../tools/test-replay-scenario.py) bind the commands
and the mask ceiling before the runner starts. The
[scenario tests](../crates/afsplus-check/tests/scenario.rs) compare a version-8
run against the same plan with observation disabled, block operation by block
operation and image block by image block.

Admission covers every code 23 through 64: the synthetic wire admits one
canonical record per kind and refuses it under an empty mask, so a kind the
host scenarios leave unproduced has an enforced wire contract. Scenario
production covers the allocator, the mutable trees, reclaim, mount recovery
with intent-log replay, a refused mount, formatting, verification phases,
staged window writes, the intent-group data barrier and read-only view
descents.

Deliberate limits: this profile observes a host memory image, and a native
adapter has its own qualification. The memory runner injects no device error
and no provider unwinding, so `TreeIoFailed`, `ReclaimFailed`, `FormatFailed`,
`DataWriteFailed`, `VerifyFailed`, `ViewReadFailed` and
`ViewMaintenanceFailed` reach the wire through the synthetic controls alone.
A healthy image reports no invariant violation, so `VerifyFinding` belongs to
the same set. The category mask selects what a batch retains; it makes no
claim about which operations produce which events.

## Linked namespace replay bundles

Semantic JSON version 9 uses `AFSPSC09`. It inherits the version-7 command set,
geometry, cache profile, diagnostic policy (category masks 0–127 and `AFSFLT05`),
snapshot limits, persistent-snapshot formatting and `expected_snapshots`
contract. [Version 8](#subsystem-and-lifecycle-replay-bundles) belongs to a separate
profile with its own category ceiling and artifact.
Earlier versions keep their exact command, observation and bundle bytes.

Labels name directory entries; a hard link receives its own label for the same
object. JSON admission checks label kinds before compilation, and the Rust parser
independently checks version, arity, hex fields, integers and ranges.

| JSON operation | Wire command | Core call | Admission |
|---|---|---|---|
| `link`: `label`, `source`, `parent`, `name` | `link LABEL SOURCE PARENT NAME` | `link_file` | Source is a live file label |
| `symlink`: `label`, `parent`, `name`, `target` | `symlink LABEL PARENT NAME TARGET` | `create_symlink` | UTF-8 target of 1–1,024 bytes without NUL, counted in the operation payload |
| `clone_file`: `label`, `source`, `parent`, `name` | `clone_file LABEL SOURCE PARENT NAME` | `clone_file` | Source is a live file label |
| `clone_range`: `source`, `source_offset`, `destination`, `destination_offset`, `length` | `clone_range SOURCE OFFSET DESTINATION OFFSET LENGTH` | `clone_range` | File labels; each offset plus length at most 16 MiB |
| `set_protection`: `label`, `protection` | `set_protection LABEL VALUE` | `set_object_protection` | Live non-root label; unsigned 32-bit value |
| `unlink_symlink`: `label` | `unlink_symlink LABEL` | `unlink_symlink` | Symlink label |
| `rename_replace`: `label`, `victim`, `parent`, `name` | `rename_replace LABEL VICTIM PARENT NAME` | `rename_replace` | Two distinct live file labels; the runner checks that the victim label names `parent`/`name` |
| `rename_replace_orphan`: same fields | `rename_replace_orphan LABEL VICTIM PARENT NAME` | `rename_replace_orphan_target` | As above; the victim label becomes an orphan label |
| `orphan_file`: `label` | `orphan_file LABEL` | `orphan_file` | Live file label; it leaves the namespace and becomes an orphan label |
| `cleanup_orphan`: `label` | `cleanup_orphan LABEL` | `cleanup_orphan` | Orphan label |
| `preallocate`: `label`, `offset`, `length` | `preallocate LABEL OFFSET LENGTH` | `preallocate_file` | File label; offset plus length at most 16 MiB |
| `preallocate_bounded`: adds `max_blocks`, `max_records` | `preallocate_bounded LABEL OFFSET LENGTH BLOCKS RECORDS` | `preallocate_file_bounded` | Budgets of 1 to 4,096 each |
| `set_data_policy`: `label`, `policy` | `set_data_policy LABEL VALUE` | `set_file_data_policy` | File label; boolean policy |
| `restore_metadata`: `label`, `protection`, `created`, `modified`, `changed` | `restore_metadata LABEL PROTECTION CREATED MODIFIED CHANGED` | `restore_object_metadata` | Live non-root label; unsigned 32-bit protection and three second stamps of 0 to 2^31 |
| `reclaim_step` | `reclaim_step` | `reclaim_step` | None |
| `snapshot_maintenance_step` | `snapshot_maintenance_step` | `snapshot_maintenance_step` | None |
| `batch`: `items` | `batch ITEM,ITEM` | `run_batch` | One to sixteen members; no member names a label the same group created |
| `window_batch`: `items` | `window_batch ITEM,ITEM` | `window_op` | As above, with `create` and `rename` members |

A batch member is one colon-separated field: `create:LABEL:PARENT:NAME:DATA`,
`delete:LABEL`, `rename:LABEL:PARENT:NAME` or
`replace:LABEL:VICTIM:PARENT:NAME`. Names and data are hex. A `batch` is one
atomic transaction and one checkpoint; a `window_batch` stages its members in
the deferred window, where `window_fsync` acknowledges every staged group and
`window_commit` publishes them.

The version-9 geometry line ends with two further fields, `DATA_POLICY` and
`ORPHAN_EXTENTS`: the `COMPAT` data-policy feature of the formatted volume as
`0` or `1`, and the orphan cleanup budget of 1 to 64 whole extent records, which
the runner reapplies at every mount. The scenario JSON carries them as
`data_policy` and `orphan_extents`, and carries `expected_orphans` with `count`
and `bytes`.

Names and targets are hex on the wire. `rename` moves any non-root label,
including a directory. The core decides block alignment, same-object CloneRange,
directory cycles and name collisions; its refusal is a captured operation failure
with retained evidence.

`AFSOBS05` keeps the `AFSOBS04` cache, run, checker and captured records and
replaces the file/directory records with a linked namespace. Recovered inspection
lists each path once and refuses a repeated directory, an internal object or an
exhausted 1,024-entry, depth-64 or 16 MiB budget. It reads opaque symlink targets
without following them:

```text
orphans COUNT BYTES
directory PATH LINKS PROTECTION
file PATH LINKS PROTECTION ALIAS POLICY DATA ALLOC
symlink PATH LINKS PROTECTION TARGET
```

The `orphans` record follows `recovered-check`. `COUNT` is the reserved
directory's entry count from `orphan_count`, and `BYTES` sums the sizes of the
objects the executed case placed there that the remounted volume names. A
reserved entry outside that set, or a failed reading, becomes
`orphans error HEX` and a case failure. `POLICY` is the persistent per-file
data-update policy as `0` or `1`. `ALLOC` is the normalized coverage described
in [generated operation families](fuzzing.md#generated-operation-families):
`-` for a file with no mapped block, otherwise comma-separated
`OFFSET:LENGTH:UNWRITTEN` intervals. A captured entry of a version-9 bundle
carries the same coverage field in place of the version-7 range count and its
`range` records. Actual JSON version 5 adds `orphans`, `orphan_error`, and a
`policy` and `alloc` field on every file entry; an expected file entry carries
`policy` and `alloc`, where `alloc` may be `null` to leave that file's layout
uncompared.

Records are sorted by path. `ALIAS` is the zero-based index of the first record
naming the same file object, so every hard link of that object repeats the index.
Data and targets are hex; empty file data is `-`. Actual JSON version 5 carries
these records as `entries`; expected version-9 entries use the same fields.
Directories have `path`, `kind`, `links` and `protection`; files add `alias` and
hex `data`; symlinks add a text `target`. The decoder refuses unordered paths,
zero link counts, noncanonical integers, forward aliases and an alias whose first
record differs in link count, protection or bytes. Success requires the exact
linked namespace, the captured registry and clean structural checks. Replay,
reduction signatures and cache binding follow version 7.

Run [admission tests](../tools/test-replay-scenario.py),
[replay tests](../tools/test-afsptest.py) and
`cargo test -p afsplus-check --test scenario`. They require version gating,
admission refusal, four-profile replay of aliases, targets, protection, clone and
unaligned-range bytes, captured failures for invalid linked operations and
failure verdicts for a wrong link count, protection, target, byte, alias,
policy flag, coverage interval or reserved-directory total.
[Generated operation families](fuzzing.md#generated-operation-families) drive
these commands with independent oracles.

## Selected-category and live-delivery bundles

Semantic JSON version 4 includes the version-3 cache and `flight_capacity`
requirements, plus `flight_categories` (integer 0–15) and `flight_sink`. Category
bits are transaction=1, checkpoint=2, I/O=4, error=8. Zero selects no events but
preserves sequence and attempt endpoints. `flight_sink` is null for no adapter,
or an object with `capacity` (1–256) and `disconnect_before` (null or 0–1024).

The deterministic consumer empties its bounded queue before each operation,
then disconnects before the designated zero-based operation index. An index
beyond the scenario length has no effect, allowing minimization to retain its
original policy. Transport processing occurs outside the filesystem operation;
callbacks use nonblocking queue admission. This is a reproducible diagnostic
consumer fixture, not a model of arbitrary external scheduling or a network trace.

The `AFSPSC04` geometry line appends category mask, sink capacity and disconnect
index after the version-3 fields. Disabled sink capacity is zero; absent disconnect
is the literal `none`. A zero-capacity sink with a disconnect index is rejected.
All scenario, image, operation and retained-event limits also apply to this profile.

`flight-recorder.bin` uses `AFSFLT03`. The 28-byte header consists of the magic,
u32 operation count, u32 ring capacity, u32 category mask, u32 sink capacity and
u32 disconnect index (0xffffffff for absent). Each original 29-byte operation
record is followed by a 53-byte batch header: cumulative dropped, filtered,
sequence endpoint, attempt endpoint, delivered and missed counts (six u64s),
closed (u8), retained count (u32). Retained events use the unchanged 26-byte
layout and kind codes from [version 3](#internal-diagnostic-bundles). Integers
are little-endian and booleans are exactly zero or one.

Admission binds the header to the scenario and checks monotonic endpoints and
counters, selected kinds, ordered event identities and retained bounds. Per
operation, generated events are the sequence-endpoint increase; selected events
subtract the filtered-count increase. The retained count must equal the lesser
of selected events and ring capacity, and the dropped-count increase must equal
selected minus retained. Overwritten events precede the first retained event.
These equations also cover empty batches and fully filtered trailing events.

Live-delivery increases are derived independently from selected count, queue
capacity and the disconnect index: before disconnection, accepted is the lesser
of capacity and selected; the rest is missed. After disconnection all selected
events are missed. Closed becomes true only after an attempted delivery to the
disconnected consumer. A disabled sink has zero delivery counts and false Closed.
No persisted counter is an attestation; exact replay additionally checks the
source/executable identity, actual artifact bytes and independent semantic oracle.

The failure signature binds cache, ring capacity, mask and complete sink policy.
Minimization retains that policy, and selected-cut bundles retain the original
full recording under the same durability-evidence limits as version 3. The
[scenario tests](../crates/afsplus-check/tests/scenario.rs) compare 640
profile/mask/capacity/consumer combinations against version-3 I/O and images.
[Replay tests](../tools/test-afsptest.py) exercise fresh processes, byte-preserved
bundles, malformed fields, counter conservation, minimization and selected cuts.
API-wide identities and additional internal subsystems have separate coverage
requirements; an empty remount batch does not establish recovery instrumentation.

## Internal diagnostic bundles

Semantic JSON version 3 requires the version-2 `volume.tree_cache_pages` policy
and a top-level integer `flight_capacity` from 1 through 256. Its `AFSPSC03`
wire header adds that capacity after the cache profile. Version 1 and 2 scenarios
keep their previous bytes and disabled internal recording. Version 3 execution
uses `Plan::run_with_flight`; the in-process API also permits diagnostic capture
for earlier scenario plans without changing their serialized contracts.

The core ring is drained after each semantic operation while retaining its
sequence, attempt identity and cumulative dropped count. It is transferred across
explicit remounts. Initial mount, remount-time recovery and final inspection are
not yet internally instrumented; an empty batch cannot establish their coverage.
The operation index, resolved object and block-log range in the enclosing semantic
record correlate retained commit events with the initiating operation. Internal
attempt IDs are not yet API-wide filesystem transaction identities.

`flight-recorder.bin` uses `AFSFLT02`: eight magic bytes, a little-endian u32
operation count and a u32 ring capacity. Each operation retains its original
29-byte `(operation:u32, first:u64, end:u64, object:u64, success:u8)` record,
followed by cumulative dropped events (u64), retained count (u32), and that many
26-byte `(sequence:u64, attempt:u64, generation:u64, kind:u8, requires_remount:u8)`
records. Kind codes 1 through 7 denote begin, queued-data completion, metadata
barrier completion, publication begin, checkpoint barrier completion, adoption
and failure. The integer encoding is little-endian; booleans are exactly 0 or 1.
At most 1024 operations and 256 retained internal events per operation are admitted.
This bounds captured diagnostics separately from image, block-log and core memory.

Bundle admission binds the capacity to the scenario and rejects unknown versions,
truncation, trailing bytes, invalid kinds/booleans/identities, nonmonotonic loss,
oversized batches and sequences inconsistent with reported overwrites. Draining
prevents already exported events from being counted as subsequently lost. Exact
replay and rebuilt comparison include the internal artifact bytes; minimization
binds the diagnostic capacity as well as cache policy in its failure signature.

A selected crash bundle retains the full original recording and its selected
block-log cut. Events after that cut describe the original recording, not actions
observed on the simulated cut device. The independent recovered-state checks
remain the crash verdict; neither a trace nor its manifest proves durability.

The [scenario tests](../crates/afsplus-check/tests/scenario.rs) verify operation
correlation across remount, failed commits, loss reporting and unchanged block
operations/image bytes at all four cache profiles. The [Python integration tests](../tools/test-afsptest.py)
exercise fresh-process private replay, preserved input bundles, resealed malformed
records, minimized failures and selected cuts. [Rebuilt-comparison tests](../tools/test-rebuilt-comparison.py)
verify distinct runner identity and refusal of changed internal events despite
passing recovered semantics; a delegating runner tests the comparison contract,
not independent build provenance. Category selection, callbacks and
other internal subsystems retain their [queue gate](../implementation/audit-work-queue.md#complete-work-queue).
