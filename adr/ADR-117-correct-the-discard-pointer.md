# ADR-117: Correct where ADR-116 sends the discard identity

Status: Accepted
Amends: ADR-116

## Context

[ADR-116](ADR-116-registry-lists-what-exists.md) removed seven identities from
the feature registry and carried a table saying, for each one, where its
subject now lives, so that nothing was lost by deletion. One row of that table
is wrong: it sends `org.aros.afsplus:discard` to
[docs/09 section 6](../docs/09-feature-framework.md), which is about
`discardable` as a property of feature state. That is a different sense of the
word. Discard as a storage operation is
[docs/07 section 7](../docs/07-allocation.md).

A decision record is immutable except for its status and relation lines
(`adr/README.md`, `AGENTS.md`), so the row is not edited in place. The rule is
what makes the log worth reading, and it holds for an author correcting their
own error as much as for anyone else.

## Decision

In the table of ADR-116, the row for `org.aros.afsplus:discard` reads
[docs/07 section 7](../docs/07-allocation.md), not docs/09 section 6.
Everything else ADR-116 decides stands.

## Compatibility classification

No format change and no code change: a cross-reference in a decision record.

## Format-change procedure

Not a format change. [docs/07 section 7](../docs/07-allocation.md) states the
subject, including that no feature identity is registered for it until its
code lands.

## Consequences

- A reader following ADR-116's table reaches the subject it names.
- The cost of this ADR is one file, and the property bought is that no
  accepted decision in this log has been quietly edited.
