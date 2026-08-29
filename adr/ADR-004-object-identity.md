# ADR-004: Object identity is independent from path

Status: Accepted

## Decision
Files and directories have stable object IDs. Directory entries map names to object IDs.

## Consequences
Rename/move preserves identity and hard links become natural. Editors and indexers can track objects robustly.
