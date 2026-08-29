# ADR-010: Feature flags instead of a linear feature version

Status: Accepted

## Decision
Ordinary evolution uses COMPAT, RO_COMPAT, and INCOMPAT feature records plus dependencies and active/enabled state.

## Rationale
A single monotonically increasing filesystem version makes cross-implementation compatibility unnecessarily fragile.
