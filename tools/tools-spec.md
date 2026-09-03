# Official Tools

The commands in this document are product tools. Development-only build and
qualification scripts are indexed separately in [README.md](README.md).

## Common command contract

Implemented commands accept `-h` or `--help`, print help on standard output
and exit 0. A diagnostic is one line on standard error:

```text
tool: error[E_IDENTIFIER]: precise context
```

The stable identifier lets tests and integrations classify a failure without
matching prose. Exit status is shared by all three tools:

| Status | Meaning |
|---|---|
| 0 | The requested operation completed and its output is valid. |
| 1 | AFS+ media, metadata or invariant failure. |
| 2 | Invalid invocation or host I/O failure, including a missing image. |

`--json` emits one UTF-8 JSON object followed by a newline. Every object has
integer `schema_version` and string `tool` members. Keys and arrays have a
deterministic order. Adding an optional member is compatible; removing a
member, changing its type or changing its meaning increments the schema.
Exact 64-bit counters are emitted as JSON integers, while bit masks and CRCs
are fixed-width lowercase hexadecimal strings. Consumers must ignore unknown
members and reject an unsupported schema version.

These are AFS+-specific operational schemas. They do not accept or pre-empt
the broader filesystem-neutral vocabulary, capability table or in-use map in
the unresolved [Tool Contract proposal](../proposals/tool-contract.md).

## mkafsplus

```text
mkafsplus [--size-mib N] [--label NAME] [--profile PROFILE]
           [--case-sensitive|--case-insensitive] [--uuid HEX]
           [--timestamp-seconds N] [--json] [--force] <image>
```

The defaults are 64 MiB, label `AFSPlus`, profile `workstation` and
case-sensitive Unicode names. `--uuid` accepts 32 hexadecimal digits with
optional hyphens. `--timestamp-seconds` supplies a reproducible signed Unix
timestamp with zero nanoseconds. Those two switches are intended for image
builders and conformance fixtures; ordinary use generates both from the host.

The formatter writes a same-directory temporary image, flushes its complete
logical length, and publishes it only after formatting succeeds. It refuses
an existing destination with `E_EXISTS`; `--force` explicitly permits
replacement. A failed format does not expose a partially initialized image at
the requested path.

The accepted profiles are `reader-minimal`, `classic-rw`, `boot-safe`,
`workstation` and `full`. The first three omit shared extents and the
persistent private-in-place data policy. `workstation` and `full` enable both.
All five enable the implemented intent-log and data-update record set.
[`profiles/`](../profiles/) is the policy source.
The formatter test gate binds its compiled feature mapping to those files so
policy and emitted identification flags cannot drift silently.

Success reports the UUID, label, block size, region size, enabled feature IDs,
name-key algorithm and requested profile. A profile is an input policy, not an
on-disk identity: the identification block records the resulting feature
masks, so an inspector must not guess which equivalent profile name was used.
The `mkafsplus` JSON schema version is 1.

## afsplus-info

```text
afsplus-info [--json] <image>
```

The bounded inspector opens the host file read-only and performs exactly the
identification read plus the two checkpoint-slot reads. It validates the
immutable identity, structurally selects the newest valid checkpoint and
reports geometry, Unicode/name policy, raw feature masks, stable feature IDs,
checkpoint roots and per-slot status. Checkpoint output distinguishes the
authoritative raw `free_blocks`, runtime `emergency_headroom_blocks` and
saturating `available_blocks` advertised to normal growth. The headroom is
computed from immutable geometry, so this adds no descendant read. The tool
therefore remains suitable for quick probes of very large volumes.

The `afsplus-info` JSON schema version is 1. It performs zero writes and uses
an OS read-only descriptor; the block interface's write and flush methods also
fail closed if future code calls them accidentally.

## afsplus-dump

```text
afsplus-dump [--json] <image>
```

The exhaustive inspector uses the same read-only backend and selected
checkpoint, then validates and emits committed objects, directory entries,
file extents, allocation-region/page summaries, referenced block sets,
reclaim runs, shared-reference runs and the valid intent-log prefix. Numeric
object/directory collections follow object ID order; entries follow on-disk
comparison-key order and block sets are numerically sorted. It reports the
same raw-free/headroom/available split as `afsplus-info`, reports the full
invariant sweep as `findings`, sets `consistent`, and exits 1 when findings
are present.

Unknown `INCOMPAT` bits stop the dump with `E_FEATURE`; guessing the layout of
unknown authoritative state would make a debugging tool misleading. Invalid
identification, checkpoint, state, extent-map and intent-log surfaces retain
distinct diagnostic IDs. The `afsplus-dump` JSON schema version is 1.

## afsplus-check

Verifier and repair tool. The implemented checker is verify-only and has
human and versioned JSON output. Planned modes are check-only, repair with
confirmation, scripted repair policy and `NO_CHANGES` forensic report.

## afsplus-resize

Initial implementation order is grow, minimum-size query, then shrink after
block relocation is proven.

## afsplus-catalog

Planned commands are `info`, `validate`, `rebuild` and `benchmark`.

## afsplus-fuse

Host mount front-end using the same portable core.
