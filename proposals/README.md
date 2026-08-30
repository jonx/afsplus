# Proposals

Drafts awaiting team review before integration. Nothing in this directory
is adopted: document and ADR numbers are assigned only when a proposal is
accepted and moved to `docs/`, `adr/`, or `spec/`.

Convention: each proposal opens with a Status line naming its target
location on acceptance, and flags the specific decisions requested from
reviewers (D*n* for API/contract questions, M*n* for media/format
questions). A rejected proposal is deleted with its reasoning recorded in
the commit message.

Current proposals:

- `tool-contract.md` — volume self-description for partition tools
  (decisions D1–D7).
- `storage-media-profiles.md` — per-medium analysis and mkfs media
  profiles (decisions M1–M5).
- `checkpoint-slot-rings.md` — wear-leveling slot rings for the checkpoint
  and intent-log areas; the format-affecting half of M1.
