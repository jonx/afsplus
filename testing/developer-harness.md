# Developer Harness

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