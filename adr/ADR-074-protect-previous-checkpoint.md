# ADR-074: Preserve the previous checkpoint and its registered snapshots

Status: Accepted
Amended by: ADR-079
Amends: ADR-036, ADR-071

## Context

ADR-036 permits promotion after retirement generation R as soon as a newer
transaction commits N > R. This protects the newest selected checkpoint while
allowing the older shadow state to lose its referenced storage. A modeled
post-snapshot-deletion write exposed that distinction: the older slot decoded
structurally, but its registry block could already be overwritten.

The owner selected stronger recovery: preserve the previous checkpoint's
metadata and registered snapshots until its slot is replaced. An extra COW
eligibility check alone cannot provide that promise; quarantine promotion must
obey the same boundary.

## Decision

Protect both structurally valid checkpoint slots during a transaction. Let P be
the minimum generation of those slots, including the newest checkpoint. A run
retired at R can leave ordinary quarantine only when R <= P. A missing or
structurally invalid older slot adds no retention beyond the valid newest slot.
Uncertainty about ownership is not permission to reclaim.

Keep the FIFO cursor before any run that fails this test, including runs inside
sealed segments and tables. A maintenance checkpoint may advance the protected
generation without promoting blocks; callers must distinguish that progress
from exhaustion. Publication and promotion retain the common crash protocol.

Force COW when a snapshot is registered in either protected checkpoint.
An unreadable older registry cannot authorize optional in-place writes. After
its slot is replaced, the older view imposes no further retention unless it is
also registered in a protected checkpoint. Runtime reader deletion rules and
persistent snapshot IDs are unchanged.

The ordinary opt-in in-place file contract from ADR-062 still allows interrupted
data updates where no registered snapshot protects the bytes. This decision
protects previous metadata and registered snapshots, not arbitrary exact-byte
history for opted-in files. It does not authorize automatic fallback from a
newer checkpoint whose descendants are corrupt: salvage must validate and
report the selected recovery source explicitly.

## Compatibility and resources

Use the existing retirement generations and checkpoint slots; introduce no
record fields, feature identities or API-v2 signatures. This strengthens the
unreleased prototype's behavioral contract. Older experimental writers are not
qualified to provide it. Require matching writer behavior, or explicit refusal,
before advertising the stronger contract across Rust/C or host adapters.

Additional quarantine consumes metadata/data headroom and can require a
maintenance publication before allocation progresses. Measure small-volume,
near-full and sustained churn behavior, promotion latency, tree/queue growth,
write amplification and peak memory. Do not invent delivery or resource bounds.

## Qualification

Reproduce post-deletion writes at every modeled cut and verify exact bytes for
snapshots registered in either valid slot. Check full reachable ownership for
both slots, including torn publication outcomes. Exercise inline, segmented and
tabled reclaim cursors stopped at protected generations, missing older slots,
reboot progress, exhausted headroom, ordinary opt-in in-place behavior and
uncertain publication. Include a negative control using the earlier promotion
rule. The checker reports damaged older state; it never silently repairs it.
