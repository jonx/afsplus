# ADR-025: Structured management API from the first release

Status: Accepted

## Context

Filesystem tooling is often difficult to automate because formatter, checker, resize, snapshot, and diagnostic tools expose unrelated command syntaxes and human-readable output that automation must screen-scrape.

AFS+ already requires a portable core and multiple front-ends. This gives us the opportunity to make automation a supported interface instead of reverse engineering CLI output later.

## Decision

Official management tools must be thin clients over stable reusable APIs where practical.

Every official tool that emits non-trivial state or progress must offer a versioned structured output mode, initially JSON where appropriate.

Examples:

```text
afsplus-info --json
afsplus-check --json
afsplus-check --json-progress
afsplus-resize --json-progress
afsplus-catalog --json
afsplus-trace --json
```

Structured schemas must include their own schema/API version.

## Rules

- human output may change for usability
- structured output changes only through explicit versioning
- progress is structured, not inferred from text
- errors have machine-readable codes and fields
- tools should expose capability discovery
- destructive operations support dry-run/planning APIs

## Consequences

Third-party partition editors, installers, recovery UIs, and automation can integrate AFS+ without parsing English text.

The portable library becomes the canonical implementation for filesystem-aware operations such as inspect, check, minimum-size calculation, and resize.