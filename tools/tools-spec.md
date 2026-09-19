# Official Tools

The commands in this document are product tools. Development-only build and
qualification scripts are indexed separately in [README.md](README.md).

<!-- toc -->

- [Common command contract](#common-command-contract)
- [mkafsplus](#mkafsplus)
- [afsplus-info](#afsplus-info)
- [afsplus-dump](#afsplus-dump)
- [afsplus-extract](#afsplus-extract)
- [afsplus-explain](#afsplus-explain)
- [afsplus-image-diff](#afsplus-image-diff)
- [afsplus-check](#afsplus-check)
- [afsplus-resize](#afsplus-resize)
- [afsplus-catalog](#afsplus-catalog)
- [afsplus-fuse](#afsplus-fuse)

<!-- /toc -->

## Common command contract

Implemented commands accept `-h` or `--help`, print help on standard output
and exit 0. A diagnostic is one line on standard error:

```text
tool: error[E_IDENTIFIER]: precise context
```

The stable identifier lets tests and integrations classify a failure without
matching prose. Exit status is shared by the commands:

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

Each schema is published as a JSON Schema (draft 2020-12) file,
[`spec/schemas/<tool>-<version>.schema.json`](../spec/schemas/), one per
tool and version: `mkafsplus-1`, `afsplus-info-1`, `afsplus-dump-1`,
`afsplus-check-5`, `afsplus-explain-2`, `afsplus-image-diff-3` and
`afsplus-probe-1` ([docs/18](../docs/18-third-party-integration.md)). A
consumer validates against the file for the version it supports; the top
level of every schema is closed, so a member this specification does not
name fails validation rather than passing unnoticed. Sections whose content
varies with the volume (`afsplus-dump` counts and diagnostics, the
per-question sections of `afsplus-explain`, the fields of a modified object
in `afsplus-image-diff`) are typed as objects and left open.
[`tools/check-json-schemas.py`](check-json-schemas.py) (`make json-schemas`)
runs every tool on the probe kit's vectors and validates each answer, with
two controls (a wrong version, an extra member) that must fail.

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
The reverse question, which profiles can take a given volume, is answered by
`afsplus-info` (below). The `mkafsplus` JSON schema version is 1.

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

`afsplus-info` also answers which compatibility profiles and implementations
can take the volume, from its feature masks and the policy files of
[`profiles/`](../profiles/) (each names every feature it allows): for each
profile `full`, `read-only` (an enabled `ro_compat` feature the profile does
not allow: it may be read but must not be written by an implementation of
that profile) or `cannot-mount` (an `incompat` one), with the features
beyond the profile named; and for the portable C reader, `reads` or
`cannot-read` from its compiled mask. A distributor formats with the
profile of the smallest implementation that must write the volume, and
checks a volume from elsewhere with this before shipping it to one.

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

## afsplus-extract

```text
afsplus-extract [--max-entries N] [--max-bytes N] <image> <new-directory>
```

Extract readable namespace objects from an offline image's selected checkpoint
using an OS read-only descriptor and `NO_CHANGES` mount. Use a stable offline
copy: concurrent writers invalidate the consistency assumption. The output
directory must be new. Original names never become host paths; file content
uses `object-ID.bin`, with original name bytes and parent/child identities
recorded in `manifest.jsonl`. Hard-linked objects are extracted once.

The manifest is streaming JSON Lines, with `schema_version: 1` and
`tool: "afsplus-extract"` on each line. Record order is header, traversal
records, summary. `link` records contain original UTF-8 bytes as `name_hex`;
`object` records contain every decoded core-record field, including timestamp
seconds/nanoseconds, protection, flags, link count and content generation.
`content` records bind object IDs to output files, recovered byte counts and
completion. Physical data-root fields are diagnostic only. File byte streams
materialize holes/unwritten extents as zeros; physical sparseness and reflink
sharing are not reproduced on the destination filesystem.

The header declares `scope: "selected-checkpoint"` and
`data_integrity: "unchecked-payload"`: payload checksums are unavailable, so
successful reads cannot establish original content integrity. Unknown feature
semantics are refused with `E_FEATURE` before output creation. Unsupported
object types and unreadable objects/directories become explicit findings;
unaffected siblings continue. A damaged directory can hide all its descendants.
Unreachable objects and damaged mount roots require additional salvage methods.

Durable log records excluded from the checkpoint produce `E_PENDING_LOG`;
an undecodable nonzero log tail produces `E_LOG_TAIL`. This extraction does
not replay the log. Retain the source image for subsequent recovery.

Defaults cap namespace links at 100,000 and extracted bytes at 1 GiB. Explicit
positive limits override these values. Entry-limit truncation and skipped files
produce findings. Streaming payload memory is 64 KiB; identity/queue memory is
proportional to admitted entries, plus the core's metadata-reader resources.
A data-read failure keeps the recovered prefix under `object-ID.partial` and
reports its offset. Output creation always refuses existing filenames.

Exit 0 means the declared checkpoint traversal finished without findings;
exit 1 indicates partial extraction or rejected media; exit 2 indicates usage
or host I/O failure. Require a final summary with `complete: true` before
accepting a complete traversal. Missing/truncated summaries and `.partial`
files preserve evidence of interrupted work. The manifest documents extraction;
full metadata-preserving backup/restore is qualified separately under Q11.
See the [extraction gate](../testing/extraction-qualification.md).

## afsplus-explain

```text
afsplus-explain [--json] <image> (block <number> | object <id> | path <path>
                                 | extent <id> <offset> | checkpoint
                                 | reclaim [<block>] | space <region>
                                 | feature [<id>])
```

Says what the committed state of an image believes, from a walk that shares no
traversal with the checker and reads the block codecs only. It opens the host
file read-only. `block` gives a block's allocation state, every role the
committed state gives it, what the block says about itself and a one-word
verdict; `object` and `path` give one object's fields, its comment, a symlink's
target, its data summary, the state of its security and attribute chains and
every directory entry that names it; `extent` says where one byte offset lives;
`checkpoint` gives both slots and the selected checkpoint's fields; `reclaim`
summarises the queue and, with a block, the pending run that holds it; `space`
counts one region; `feature` lists the feature bits.

The JSON schema version is 2. Every document carries the generation it
answers for, whether the volume keeps snapshots, and whether the walk could
decode every branch. Status 0 is an answer from a complete walk, 1 an answer
from a partial walk or a question about something the image does not hold, 2 a
usage or host I/O failure.

## afsplus-image-diff

```text
afsplus-image-diff [--json] [--metadata] <before> <after>
```

Reports what the second image differs by from the first in filesystem terms
rather than in blocks: objects created, removed and changed, links added and
removed, renames, orphan changes, allocation totals and, unless `--metadata`
is given, the byte ranges of changed file content. Both images are opened
read-only. The JSON schema version is 3. A reported difference is a normal
result with status 0; an image that could only be compared in part gives the
media status and names the branch in `problems`.

## afsplus-check

Verifier and repair tool. The implemented checker is verify-only and has
human and versioned JSON output; the JSON schema version is 5 (`clean`, the
volume summary or `null` when the volume could not be read, the two slot
verdicts, `warnings`, `errors`). Planned modes are check-only, repair with
confirmation, scripted repair policy and `NO_CHANGES` forensic report.

## afsplus-resize

Initial implementation order is grow, minimum-size query, then shrink after
block relocation is proven.

## afsplus-catalog

Planned commands are `info`, `validate`, `rebuild` and `benchmark`.

## afsplus-fuse

Host mount front-end using the same portable core.
