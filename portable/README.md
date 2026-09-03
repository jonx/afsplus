# Portable implementations

This directory contains implementations that consume the normative AFS+ wire
format without linking the Rust crates.

[`c/reader.c`](c/reader.c) is the executable `reader-minimal` C99 path. It owns
no memory, uses caller-supplied logical-block I/O and scratch space, validates
the identification record, selects the newest structural checkpoint, descends
typed object/directory/extent trees, decodes object records and performs
bounded regular-file reads and a validated checkpoint-plus-intent view. It
does not yet carry the Unicode tables needed to validate arbitrary non-ASCII
comparison keys. [`c/writer.c`](c/writer.c) adds the first `classic-rw` slice:
one durable non-replacing file rename in a preallocated intent slot, with
caller-owned I/O and memory. The
[embedding guide](c/README.md) documents the ABI, capabilities, memory
contract, diagnostics and host example.

Run its Rust-to-C interoperability and corruption gate with:

```sh
make portable-c-gate
make portable-c-fuzz-gate
```

The second command records the exact blocks touched by successful operations
into compact `.afzf` seeds, then mutates and replays them under sanitizers.
`make portable-c-fuzz-long` raises the deterministic run count for unattended
qualification without requiring a platform fuzzing runtime.
