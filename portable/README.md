# Portable implementations

This directory contains implementations that consume the normative AFS+ wire
format without linking the Rust crates.

[`c/reader.c`](c/reader.c) is the executable `reader-minimal` C99 path. It owns
no memory, uses caller-supplied logical-block I/O and scratch space, validates
the identification record, selects the newest structural checkpoint, descends
typed object/directory/extent trees, decodes object records and performs
bounded regular-file reads. It does not yet replay the intent log or carry the
Unicode tables needed to validate arbitrary non-ASCII comparison keys. The
[embedding guide](c/README.md) documents the ABI, capabilities, memory
contract, diagnostics and host example.

Run its Rust-to-C interoperability and corruption gate with:

```sh
make portable-c-gate
```
