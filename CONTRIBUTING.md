# Contributing

## Format changes

Any change to on-disk semantics requires:

1. specification update
2. ADR or amendment to an existing ADR
3. compatibility classification
4. conformance image
5. parser tests
6. repair-tool behavior
7. resource impact analysis

## API changes

Filesystem API v2 changes require:

- ABI/versioning analysis
- legacy compatibility impact
- capability semantics
- Rust mapping impact
- at least one filesystem-neutral test

## Performance work

Performance patches must include:

- workload
- before/after measurement
- peak memory
- read/write amplification
- invariant test results

## Rust quality gate

The repository has one root `rustfmt.toml`; format the entire workspace rather
than individual crates. Before committing Rust changes, run:

```sh
cargo fmt --all -- --check
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

## Documentation

Before committing a documentation change, run:

```sh
make check-docs
```

It verifies links and anchors, tables of contents, the ADR index, ADR and
milestone references, navigation blocks and index rows.
`make toc` regenerates the tables of contents and the ADR index.

## No private fast paths

Applications must not be taught AFS+ block layouts. Add semantic filesystem APIs instead.
