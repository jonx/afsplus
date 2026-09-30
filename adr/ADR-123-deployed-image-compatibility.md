# ADR-123: Preserve registered deployed images before the global format freeze

Status: Superseded
Amends: ADR-104, ADR-115
Superseded by: ADR-124

## Context

AFS+ serves an AROS `SYS:` volume on the M1. The owner requires preservation
of that image while development continues. A public-release criterion does
not describe this deployment. Epoch 1 has covered incompatible prototype
layouts, so keeping its integer unchanged does not establish compatibility.

## Decision

Apply the [protected deployed-image contract](../spec/compatibility-rules.md#protected-deployed-images)
to registered immutable images. Retain the current writer's complete clean
and pending-log images, their semantic oracle, and the exact deployed M1
initializer with its zero-filled suffix specified independently.

Admission, recovery and mutation changes must preserve those baselines.
Adding a baseline never replaces an older one. Retiring protected bytes
requires an explicit compatibility and migration decision, not the premise
that no public image has been released. The prototype changes recorded by
ADR-104 and ADR-115 are historical; this decision does not reinstate their
retired 96/112-byte checkpoint layouts or removed codecs.

This deployment contract does not declare the broader M14 freeze complete.
Unsupported features and corrupt metadata continue to be refused.

## Consequences

The [retained-image test](../crates/afsplus-check/tests/deployed_compatibility.rs)
reads bytes written before future changes. It checks content, metadata,
case-insensitive lookup, mutation/remount, replay, and rejection before
writes. The [manifest](../crates/afsplus-check/tests/fixtures/deployed-epoch1/manifest.json)
separates known source provenance from the unrecorded formatter commit of
the actual SYS initializer. No live M1 data is copied or changed by the gate.

No on-disk byte or parser rule changes in this decision. Resource cost is
retained test data and a targeted host test; handler size is unaffected.
