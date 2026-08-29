# ADR-001: AFS+ is a new on-disk format

Status: Accepted

## Context
Classic AFS/FFS embeds 32-bit-era file sizes, block references, name limits, and layout assumptions. Extending all of them while keeping binary compatibility would constrain the new design.

## Decision
AFS+ uses a new DOS/filesystem identity and a new on-disk format. Existing FFS volumes continue to use the existing handler.

## Consequences
Migration requires copy/restore or a dedicated converter. The new format can be designed cleanly.
