# ADR-116: The feature registry lists what exists

Status: Accepted
Amended by: ADR-117
Amends: ADR-038

## Context

`spec/feature-registry.toml` held fourteen identities. The code assigns seven
feature bits. The other seven were of three kinds, and each kind was a
different mistake.

One had no compatibility class at all: `org.aros.afsplus:data-checksums`, with
`class = "tbd"`, `rebuildable = "tbd"`, `discardable = "tbd"`. Its class cannot
be decided, because it follows from a storage design nobody has made: if
checksums live apart from the extent record and a reader that ignores them may
still write, that reader silently invalidates them and a checksumming reader
then reports corruption on good data; if they change the extent value, the
feature is incompatible. Nothing of it exists: no algorithm, no granularity,
no layout, no writer, no checker, no bit.

Two described behaviour that is not optional at all.
`org.aros.afsplus:xattrs`, class `ro_compat`, described extended attribute
storage, but [ADR-108](ADR-108-extended-attributes.md) put attributes in the
base format: the reference is an object flag every implementation must know,
and no volume feature gates it. `org.aros.afsplus:sparse`, class `incompat`,
described sparse ranges, but a hole is an absent extent that every reader
already handles, and no bit exists. A registry line calling shipped
base-format behaviour an optional feature is worse than a missing line: a
second implementer would gate the field on a bit that no image ever sets, and
refuse volumes everyone else reads.

Four were plans with a class chosen on paper and no code: `catalog`,
`change-stream`, `inline-data` and `discard`.

A registry that is meant to be frozen, and that a third implementation is
tested against, cannot mix these with the seven identities that exist.

## Decision

1. The registry lists an identity when its bit, its class and its code land
   together.
2. Base-format behaviour is not a feature and gets no identity. Every
   implementation must have it, so there is nothing to negotiate.
3. A planned feature lives in its ADR or its open question, not in the
   registry. It joins the registry with its implementation.
4. A test holds the registry to this: every identity names a bit this code
   assigns with the class its word implies, every assigned bit has an
   identity, and no bit of a word is used twice.

The registry now lists exactly the seven identities the code assigns:
`intent-log`, `intent-log-data-updates`, `persistent-snapshots` and
`security-descriptors` (INCOMPAT bits 0 to 3), `shared-extents` and
`orphan-directory` (RO_COMPAT bits 0 and 1), and `data-policy` (COMPAT bit 0).
`registry_version` becomes 6.

## Where each removed line's subject now lives

| Removed identity | Its subject now |
|---|---|
| `org.aros.afsplus:data-checksums` | [docs/06 section 7](../docs/06-files-and-extents.md): the base format leaves room for a checksum association; the feature arrives with its design, its tests and a new identity |
| `org.aros.afsplus:xattrs` | [ADR-108](ADR-108-extended-attributes.md): extended attributes are base format, behind object flag bit 4, gated by nothing |
| `org.aros.afsplus:sparse` | The base format: a hole is an absent extent of the extent map ([docs/06](../docs/06-files-and-extents.md)) |
| `org.aros.afsplus:inline-data` | [ADR-018](ADR-018-inline-data-optional.md), reopened as [Q6](../implementation/open-questions.md) |
| `org.aros.afsplus:catalog` | [Q8](../implementation/open-questions.md), developer primitives |
| `org.aros.afsplus:change-stream` | [Q8](../implementation/open-questions.md), and the change-record actor field of [ADR-103](ADR-103-change-record-actor.md) |
| `org.aros.afsplus:discard` | [docs/09 section 6](../docs/09-feature-framework.md): a hint with no on-disk correctness semantics, so it needs no identity until it has code |

## Compatibility classification

No format change: no image carries a feature bit for any removed identity,
because none was ever assigned. The registry is a specification document and
the seven surviving entries are unchanged.

## Format-change procedure

Not a format change. The rule is held by
[`feature_registry.rs`](../crates/afsplus-format/tests/feature_registry.rs),
which reads `spec/feature-registry.toml` and compares it with the constants of
`afsplus_format::ident` in both directions. Nothing else reads the registry:
it is linked by [docs/09](../docs/09-feature-framework.md),
[docs/README](../docs/README.md) and two qualification plans, and parsed by no
tool.

## Consequences

- A second implementer reads seven identities and finds seven bits.
- A feature cannot be half-registered: the line and the code arrive together.
- The pre-freeze list loses the `tbd` item.
