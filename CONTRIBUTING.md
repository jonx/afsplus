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

The repository has one root [`rustfmt.toml`](rustfmt.toml); format the entire workspace rather
than individual crates. Before committing Rust changes, run the named tests of
what changed (`cargo test -p CRATE --test FILE NAME`) and the compile and
lint checks:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

The workspace test suite is not part of this gate: see
[the development method](implementation/development-method.md) for what
counts as proof. A change to a codec also runs `make rust-codec-fuzz-gate`,
which fuzzes that codec against inputs its author did not choose.

`make rust-gate` includes the separate codec-fuzz workspace. Keeping that crate
out of ordinary component builds does not exempt it from the host quality gate.

## Documentation

Before committing a documentation change, run:

```sh
make check-docs
```

It verifies links and anchors, tables of contents, the ADR index, ADR and
milestone references, navigation blocks, index rows and the status rules.
`make toc` regenerates the tables of contents and the ADR index.

The writing rules — finished-state text, one home per fact, the navigation
block, the per-document rules and the checklist before committing — are in
[docs/DOCUMENTATION.md](docs/DOCUMENTATION.md). Where the project stands is
written only in [implementation/milestones.md](implementation/milestones.md);
the story goes to [NOTES.md](NOTES.md).

## No private fast paths

Applications must not be taught AFS+ block layouts. Add semantic filesystem APIs instead.
