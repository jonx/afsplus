# ADR-018: Tiny-file inline data is an optional future feature

Status: Proposed

## Decision
Reserve an extension path for storing tiny file data with object metadata.

## Rationale
Modern source trees contain huge numbers of small files. Inline data can reduce allocation metadata and I/O, but should not burden minimal readers until measurements justify it.
