# ADR-069: Develop consistent filesystem snapshots before individual version retention

Status: Accepted
Amends: ADR-021
Amended by: ADR-070

## Context

Q4 asks how exact older content stays readable for backup and scanning.
Retaining individual files does not establish a consistent view across names,
links, directories and multiple files. The owner selected consistent
filesystem snapshots as the first consumer-facing development direction.

## Decision

Develop an explicitly retained, consistent filesystem snapshot for the first
backup/scanner consumer before a general individual-file version-retention
facility. Record the snapshot prototype in the roadmap. Q4 continues to own
retention limits, admission and reclamation policy until measured evidence
supports an explicit follow-up decision.

A snapshot identifies one coherent committed namespace and its corresponding
file content. Live mutations cannot change bytes observable through that
snapshot. An in-place data optimization must therefore be disabled or made
COW for any range whose old bytes the snapshot protects, even if that range
has only one live mapping in the active checkpoint.

Snapshot enumeration and reads use filesystem-neutral semantic capabilities.
Consumers must receive explicit failure if the requested snapshot cannot be
created or retained. Space pressure must be observable and must not silently
invalidate an actively promised view.

## Prototype and acceptance evidence

Use a backup/scanner consumer that enumerates and reads while the live tree
creates, writes, truncates, renames, links and deletes objects. Compare the
snapshot's namespace, metadata and complete bytes with an independent oracle
captured at its commit boundary. Include opted-in in-place files and reflinks.

Bound and measure RAM, metadata/data amplification, pinning and reclaim work.
Test admission near ENOSPC, release and subsequent reclamation, reboot and
interrupted publication/deletion. Distinguish runtime-only pins from snapshots
promised to survive remount; the latter need a reviewed persistent ownership
representation and crash corpus before being advertised.

## Boundaries and compatibility

This decision selects development priority and semantic requirements. It adds
no feature ID, disk field, checkpoint slot, retention duration, public API
signature or implicit automatic-snapshot policy. The format and API change
procedures apply to the resulting prototype before those interfaces stabilize.
Individual version access may later build on snapshots, with explicit
unavailable results when the requested view is not retained.
