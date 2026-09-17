# ADR-115: A version or block kind nothing writes is refused, not read

Status: Accepted
Amends: ADR-036, ADR-038

## Context

Three prototype layouts survived in the readers with nothing to read. The
identification block admitted versions 1 and 2 beside the version 3 the
formatter writes; the intent-log record admitted version 0, which has no
operation timestamp, beside versions 2 and 3 the writer emits; and three block
kinds of the first milestones still had codecs, fuzz targets and magics:
`"AFSD"`, the one-block directory that the typed COW tree replaced, `"AFSM"`,
the flat object map, and `"AFSR"`, the retired list that
[ADR-036](ADR-036-reclaim-queue.md) already described as "transitional test
coverage only".

No image of this filesystem has been released, so there is nothing to be
compatible with. The question is only whether a reader should accept a shape
no writer produces. It should not: the shape is admission surface that no
image needs, that no test outside the codecs' own exercises, that a second
implementer must reproduce to be conformant, and that a fuzzer explores
instead of exploring what volumes contain.

A survey came first, because removing admission is irreversible for any image
that does hold such a block. Nothing does. No crate outside the format
crate's own tests builds one. Every file of the repository and all of `build/`
in three worktrees, 78,500 files, were scanned for a block magic at a 4 KiB
boundary: no `"AFSD"`, `"AFSM"` or `"AFSR"` image, no identification block of
any version, and no intent-log record of a version other than 2 or 3. The
retained fuzz regressions hold only a README, and the four static fixtures are
checkpoint blocks. No tool or document tells anyone to read one.

## Decision

1. The identification block has one version, 3. Versions 1 and 2 are refused
   as versions, before the payload length is judged, because the version is
   what states the length.
2. The intent-log record has versions 2 and 3, the two the writer emits.
   Version 0 is refused.
3. `"AFSD"`, `"AFSM"` and `"AFSR"` are not block kinds of this format. Their
   codecs, fuzz targets and block-type constants are removed.
4. A retired version number or block magic is never reused. A later layout
   takes the next free number, so a block of a retired kind or version stays
   what it is: refused.
5. Refused means refused, not ignored. A reader that meets one does not skip
   it, treat it as empty, or read the part it recognises.

## Compatibility classification

Whole-format change without a feature identity: no image of this filesystem
has been released, and the survey above found none to refuse. No written byte
changes.

## Format-change procedure

| Step | State |
|---|---|
| Specification update | [Directory entries](../crates/afsplus-format/src/dir.rs) and the fuzz target list of [fuzzing](../testing/fuzzing.md) |
| ADR | This ADR |
| Compatibility classification | Whole-format change that narrows admission only, reasoned above |
| Conformance image | None; the survey found no image of a retired version or kind anywhere |
| Parser tests | [`retired_surface.rs`](../crates/afsplus-format/tests/retired_surface.rs): identification versions 0, 1, 2 and 4, each at its own prototype payload length and at the current length, so the version and not the length decides; intent-log versions 0, 1 and 4; and, for the three magics, that no current kind claims one and that a well-formed block carrying one verifies as no current kind |
| Repair-tool behavior | The checker and `afsplus_check::explain` no longer know the three magics: such a block reports no identity and, if allocated, appears as owned by nothing |
| Resource impact | Three codecs, three fuzz targets and two version branches removed |

Assertions that moved, none because an image changed: the round-trip test lost
the eleven cases that existed only to exercise the retired codecs and versions;
`legacy_reserved.rs` and the legacy fuzz oracle were deleted with them. The
three retired fuzz targets held IDs 19 to 21, the last of the enum, so every
surviving row of the pinned fingerprint table is unchanged and the seed schema
stays at 2. One dead branch in an unrelated test went with them: the explain
command's test looked for an `"AFSD"` root directory that no volume writes.

## Consequences

- A second implementer has three fewer kinds and three fewer versions to
  reproduce.
- `afsplus-format` no longer carries `omap.rs` and `retired.rs`; `dir.rs`
  keeps the directory entry and its tree-leaf codec, which are live.
- The pre-freeze list loses one item.
