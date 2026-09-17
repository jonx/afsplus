# Proposals

Drafts awaiting team review before integration. Nothing in this directory
is adopted: document and ADR numbers are assigned only when a proposal is
accepted and moved to `docs/`, `adr/`, or `spec/`.

Convention: each proposal opens with the navigation block and a
"Target on acceptance" line naming where it lands if accepted, and flags
the specific decisions requested from reviewers (D*n* for API/contract questions, M*n* for media/format
questions). A rejected proposal is deleted with its reasoning recorded in
the commit message.

Proposals:

- [`tool-contract.md`](tool-contract.md) — volume self-description for partition tools
  (decisions D1–D7).
- [`storage-media-profiles.md`](storage-media-profiles.md) — per-medium analysis and mkfs media
  profiles (decisions M1–M5).
- [`checkpoint-slot-rings.md`](checkpoint-slot-rings.md) — wear-leveling slot rings for the checkpoint
  and intent-log areas; the format-affecting half of M1.
- [`filesystem-api-v2-abi-freeze.md`](filesystem-api-v2-abi-freeze.md) — additive, portable
  filesystem API v2 ABI and AROS handler transport (decisions D1–D8).

- [`persistent-snapshot-prototype.md`](persistent-snapshot-prototype.md) — Q4 registry, retention accounting, mutation isolation and admission experiments (S1–S4).
- [`adr-clone-metadata-inheritance.md`](adr-clone-metadata-inheritance.md) — Q14: what CloneFile and CloneRange inherit, and an untouched source (decisions D1–D4).
- [`adr-security-preservation-container.md`](adr-security-preservation-container.md) — B5 and the format half of Q5: opaque versioned security descriptors, the object security reference and the projection rule (decisions M1–M5).
- [`adr-object-record-admission.md`](adr-object-record-admission.md) — Q13: exact admission of object header flags, payload length and unused tail (decisions M1–M3).
- [`adr-change-record-actor.md`](adr-change-record-actor.md) — reserving an advisory
  host-supplied actor field in the epoch-1 change record (decisions M1–M4, D1).
