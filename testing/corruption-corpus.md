# Checker Corruption Corpus

> **ADRs:** [ADR-025](../adr/ADR-025-structured-management-api.md) · **Spec:** [invariants](../spec/invariants.md) ·
> **Tests:** none · **Milestones:** M05

The checker corruption corpus is a deterministic, independently replayable
set of damaged AFS+ images. It verifies that malformed media produces a
bounded diagnostic rather than a panic, silent acceptance or an accidental
fallback to unrelated state.

## Gate

```sh
cargo test -p afsplus-check --test corruption_corpus
```

The test builds every image twice and requires byte-identical images,
manifests and checker reports. It runs each case through `check_device` behind
an unwind boundary, repeats the check to prove stable JSON output, and checks
that all six implemented wire surfaces have both an integrity and a semantic
case.

## Generate replay artifacts

```sh
cargo run -p afsplus-check --bin afsplus-corruption-corpus -- /tmp/afsplus-corruptions
```

The destination must not exist. The generator never replaces caller data. It
writes:

- one sparse 1 MiB image per case;
- one exact `afsplus-check` JSON report per image;
- `manifest.json`, binding each stable case ID to its surface, byte-level
  mutations, expected channel and exact diagnostic; and
- a short standalone replay instruction.

Replay one image with:

```sh
cargo run -p afsplus-check --bin afsplus-check -- \
  /tmp/afsplus-corruptions/object-zero-link-count.img --json
```

Consumers reject unknown `corpus_schema_version` or
`checker_schema_version` values. The corpus schema versions the artifact
contract; the embedded checker schema versions the report contract from
[ADR-025](../adr/ADR-025-structured-management-api.md).

## Matrix

| Surface | Integrity case | Valid-CRC semantic case | Expected result |
|---|---|---|---|
| Identification | payload/checksum mismatch | unknown INCOMPAT bit | hard error |
| Checkpoints | both retained checksums invalid | reserved payload flags set | no selectable checkpoint |
| Typed tree | object-map checksum invalid | leaf item accounting overstated | chosen state rejected |
| Object record | mapped record checksum invalid | zero link count | chosen state rejected |
| Allocation bitmap | selected page checksum invalid | reachable object block marked free | chosen state rejected |
| Intent log | nonzero record checksum invalid | first sequence is two | clean crash boundary plus forensic warning |

Intent-tail damage is intentionally a warning: the first invalid record ends
the durable prefix and is a legal crash artifact. A nonzero undecodable block
is distinguished from a zero unused slot so forensic tooling can report what
ended the prefix without treating an incomplete `fsync` as committed data.

## Adding a case

Add the mutation to `afsplus_check::corpus::build_corruption_corpus` with a
permanent lowercase case ID. Record every changed LBA and byte range, prefer a
valid-checksum semantic mutation over a redundant checksum flip, and pin the
exact expected diagnostic. The case is complete only when the checker result
is bounded, deterministic, reproduced by the exported image, and documented
in the matrix. A diagnostic wording change is therefore an intentional corpus
contract change rather than an unnoticed test drift.
