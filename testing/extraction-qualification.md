# Read-only extraction qualification

> **ADRs:** none · **Spec:** [tool contract](../tools/tools-spec.md#afsplus-extract), [invariants](../spec/invariants.md) ·
> **Tests:** `cargo test -p afsplus-tools` · **Milestones:** M05, M13

The [extractor](../crates/afsplus-tools/src/extract.rs) exports readable objects
from a selected checkpoint into a new destination. Its contract and limitations
are defined in the [tool specification](../tools/tools-spec.md#afsplus-extract).
Q11 owns additional damage classes, repair and complete restoration.

## Executable gate

Run `cargo test -p afsplus-tools`. The CLI corpus creates independent temporary
images and output directories. It verifies:

- exact multichunk file bytes, sparse logical zeros and hard-link identity;
- original quoted/Unicode name bytes, timestamp precision and core metadata;
- byte-identical source and unchanged modification time after extraction;
- refusal of existing destinations, including the source image pathname;
- a damaged object record producing an explicit object finding while a healthy
  sibling's complete bytes are recovered;
- a damaged nested directory producing a subtree finding while a healthy
  sibling is recovered, followed by hard refusal of a damaged mount root;
- explicit entry/byte limits and refusal of unknown feature semantics before
  destination creation.

A generic-device unit fixture permits reads and panics on any write or flush
attempt, starting before `NO_CHANGES` mount. Its file crosses the 64 KiB
streaming boundary; an injected read failure after that boundary must leave
exactly the old checkpoint prefix in a `.partial` file. A durable pending log
write must be reported as excluded. The summary must report both findings.

## Required extensions

Qualify each additional salvage class with an exact corruption/extraction oracle.
Raw object discovery needs bounded scanning, confidence/provenance records and
an explicit policy for conflicting generations. Recovery from damaged object-map
or allocation roots must preserve this tool's no-source-write contract.

A complete restore consumer must preserve sparse semantics, timestamps, links,
attributes and security metadata through accepted APIs. The extraction manifest
retains diagnostic fields but cannot grant unsupported restore capabilities.
Snapshots supply online consistency once the persistent view gate passes.
