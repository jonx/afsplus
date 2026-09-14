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
