# Exact admission for the reclaim queue blocks

> **ADRs:** none · **Spec:** none ·
> **Tests:** none · **Milestones:** none

Target on acceptance: a numbered ADR. The change is format-affecting in the
sense of [ADR-100](../adr/ADR-100-exact-object-record-admission.md): it narrows
what a reader admits and changes no byte any writer produces. Decision
requested: R1 below.

## Context

While giving the reclaim root, segment and table a second reader
(`afspr_decode_reclaim_*` in the portable C reader, cross-read in
[`reclaim_c.rs`](../crates/afsplus-format/tests/reclaim_c.rs)), five things
turned out to be outside admission in both readers:

1. nonzero flags in the common header;
2. a nonzero owner in the common header, though the queue belongs to the
   volume and the encoder writes zero;
3. bytes in the unused slots of a root area, beyond the area's count;
4. a root payload longer than its three areas;
5. bytes after the payload, in all three kinds.

A sealed segment or table already has an exact payload length. The encoders
write a zeroed block, so every image this filesystem has produced satisfies
all five. The object record was in the same state before ADR-100, and the
argument there applies: the root is rewritten by every transaction from its
decoded fields into a zeroed block, so any byte admitted without a field is
dropped by the next commit, and a byte that is dropped silently is a byte two
implementations can disagree about.

## Proposal

Refuse all five in every reader: zero header flags, zero owner, zero unused
slots, a root payload of exactly 64 + 12 × (table + segment capacity) + 20 ×
inline capacity, and a zero tail.

## Decision requested

R1. Adopt the rule before the wire freeze (M14), or record that the reclaim
blocks keep tolerant admission and why.

## Cost

Five comparisons in the Rust decoders, five in the C decoders, five in the
independent fuzz oracle (`fuzz/src/reclaim.rs`), and the five "admitted
today" image families of the cross-read test turn into refusals. No fixture,
conformance image or corpus entry changes: none carries such bytes.
