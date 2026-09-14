# Developer Harness

> **ADRs:** none · **Spec:** none ·
> **Tests:** none · **Milestones:** M05

<!-- toc -->

- [Goal](#goal)
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

[replay-scenario.py](../tools/replay-scenario.py) admits a version-1 JSON scenario
and compiles its operation list to the fixed ASCII `AFSPSC01` protocol consumed
by [the memory runner](../crates/afsplus-check/src/scenario.rs). Names and data are
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
retained for reproduction; dirty-source reconstruction and build provenance need
separate evidence.

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
