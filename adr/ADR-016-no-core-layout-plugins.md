# ADR-016: Fundamental layouts are not runtime plugins

Status: Accepted

## Decision
Objects, directories, extents, and allocation have standardized core layouts. Only narrow feature providers may be pluggable.

## Rationale
Making every fundamental algorithm replaceable creates an implementation and compatibility matrix that is hostile to small ports.
