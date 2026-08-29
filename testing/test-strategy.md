# Test Strategy

## Test pyramid

### Unit

Encoding, checksums, normalization, tree operations, allocation arithmetic, extent merging.

### Property

Examples:

- write/read round-trip
- rename preserves object ID
- create then unlink returns free-space count
- directory iteration returns every reachable entry exactly once
- transaction abort leaves authoritative state unchanged

### Image conformance

Known byte-for-byte images with expected output.

### Crash

Interrupt writes at every modeled persistence point.

### Fuzz

Mutate every metadata parser.

### Interoperability

Same image opened by:
- host reader
- FUSE
- AROS handler
- checker

### Application

Git, Cargo, Zed-style watcher, Ferail, Moonstone.

## Reproducibility

Every failure records:
- seed
- image hash
- implementation revision
- active feature set
- operation trace
