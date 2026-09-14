# ADR-070: Make the first consistent snapshots persistent

Status: Accepted
Amends: ADR-069
Amended by: ADR-071

## Context

ADR-069 selects consistent filesystem snapshots before individual version
retention. The owner additionally selected persistence across reboot for the
first snapshot prototype, rather than runtime-only pins.

## Decision

The first snapshot facility targets persistent, explicitly retained views.
Snapshot creation, registry publication, deletion and release of protected
storage must be crash-safe. An acknowledged snapshot is discoverable and
readable after remount with the same namespace, metadata and file bytes.
Deleting a snapshot cannot free storage still reachable from another snapshot,
a live mapping or a selectable checkpoint.

Prototype admission and retained-space reporting before fixing retention
limits. An inability to create or maintain a promised view must be explicit;
no policy may silently expire a view whose lifetime was promised to a caller.
Automatic snapshot schedules, expiry and quotas are separate policy choices.

## Architectural work and validation

The existing triple-version allocation-root pool supports two selectable
checkpoints and a new commit. Persistent snapshots must not simply pin
arbitrary old allocator roots in that pool: doing so invalidates its capacity
proof. Evaluate a persistent namespace-root registry with independent retained
ownership accounting against alternatives, using the common COW engine.

The prototype must cover registry creation/deletion cuts, interrupted recovery,
multiple retained views, live in-place opt-in files, reflinks, ENOSPC and
bounded reclamation after release. Read snapshots after remount and after many
live commits against an independent immutable oracle. Report retained physical
space separately from reclaimable and immediately available capacity.

## Compatibility boundary

This decision adds no disk fields or public ABI. A registry encoding and its
compatibility class require a follow-up ADR, Rust/C conformance corpus,
checker ownership rules and resource evidence before implementation acceptance.
Q4 owns the remaining representation, retention and admission decisions.
