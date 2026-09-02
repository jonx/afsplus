# Portable implementations

This directory contains implementations that consume the normative AFS+ wire
format without linking the Rust crates.

[`c/reader.c`](c/reader.c) is the first `reader-minimal` C99 slice. It owns no
memory, uses a caller-supplied logical-block callback and one caller-supplied
scratch block, and independently validates the identification record and
structurally selects the newest checkpoint. It does not replay the intent log
or expose filesystem objects until those layers have their own
cross-implementation tests. The [embedding guide](c/README.md) documents the
ABI, CMake target, callback contract, diagnostics and host example.

Run its Rust-to-C interoperability and corruption gate with:

```sh
make portable-c-gate
```
