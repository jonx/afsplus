# ADR-124: Prefer coordinated format changes over pre-release compatibility debt

Status: Accepted
Supersedes: ADR-123

## Context

The owner sets a standing rule: before the first official release, internal
images and earlier implementations do not justify compatibility debt. If a
break is the better design, change every consumer and tool together. This
overrides the preservation promise in ADR-123 without discarding its useful
regression tests or its observed SYS-image provenance.

## Decision

Apply the [pre-release format rule](../spec/compatibility-rules.md#pre-release-format-changes).
Record intentional breaks in an ADR, select appropriate epoch/version/feature
admission, and update specification, producers, readers, adapters, checkers,
tools and affected conformance images in one coordinated change. No legacy
reader or migration layer is required solely for our internal earlier work.
This rule ends with the first official release.

Retained image bytes are reference test inputs. They detect unintentional
semantic or admission changes, including changes that would affect SYS.
They are not a frozen format contract: a reviewed intentional break updates
their bytes, hashes, provenance and expected semantics together. Silently
refreshing a fixture to suppress an unexplained failure is not such a decision.

## Consequences

The clean, pending-log and M1-initializer tests continue to run without a
binary change. Their checks remain useful against the chosen implementation;
they make no indefinite historical-reader promise. This decision changes
policy only and neither migrates nor modifies the live M1 image. Physical
disk and macOS protections remain separate from format-design choices.
