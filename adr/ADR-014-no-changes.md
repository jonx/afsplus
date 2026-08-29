# ADR-014: Strict no-changes mount mode

Status: Accepted

## Decision
AFS+ defines a mount mode that performs zero media writes, including journal replay and housekeeping.

## Rationale
Forensics, recovery, compatibility testing, and debugging require a stronger guarantee than ambiguous read-only behavior.
