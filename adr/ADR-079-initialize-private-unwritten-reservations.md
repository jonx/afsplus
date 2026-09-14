# ADR-079: Initialize private unwritten reservations before publishing written mappings

Status: Accepted
Amends: ADR-062, ADR-074, ADR-078

## Context

Preallocation owns physical storage while its unwritten mappings read as zeros.
The reference write path allocates replacement blocks even for these mappings.
That needs extra data capacity when an application starts consuming its reserved
space. The owner authorized recommended decisions while away and requires
constrained-system behavior to be qualified.

## Decision

Permit initialization of a touched private unwritten allocation at its existing
physical address. Publish the written mapping through the common COW metadata
transaction only after the initialized data satisfies the durability barrier.
Reconstruct complete touched blocks from logical zeros and caller bytes; never
expose stale physical contents from the reservation.

Eligibility requires an explicitly unwritten mapping with no shared marker and
no overlapping live reference-tree record. An inconsistent private marker with
a shared record is corruption and must be refused. Shared or conservatively
marked reservations use fresh storage. Written ranges and holes retain their
existing COW or explicitly opted-in in-place contracts. A mixed write may
initialize eligible reserved portions while copying the other portions.

The allocation stays owned, with its existing lifetime birth and bitmap state.
Do not retire or reallocate initialized blocks. Every retained older unwritten
mapping continues to return logical zeros without reading the physical payload;
metadata publication determines when the new written bytes become visible.
Written mappings must never be converted back to unwritten in place. Ordinary
release, quarantine and snapshot retention protect a written allocation before
it can be allocated as a new reservation.

This preserves exact old-or-new logical bytes through crashes, including when
an older checkpoint or snapshot maps the initialized blocks as unwritten.
Initialization does not authorize overwriting data that any retained written
mapping can observe. Keep the written-data in-place policy separate and report
initialized-reservation blocks separately in commit counters.

## Alternatives and limits

Always allocating replacement storage preserves the content contract but does
not consume the owned reservation directly. Removing reservations would violate
the full-preservation direction. Supporting every shared-unwritten case would
require proving the semantics of every peer and retained written mapping;
conservative fresh allocation gives a bounded first eligibility rule.

Metadata publication still needs headroom. Preallocation does not guarantee
success under metadata exhaustion, device failure, revocation or unsupported
provider behavior. Bounded extent traversal and resource profiles require their
own qualification; this decision does not claim them complete.

## Compatibility and qualification

Record encodings and feature identities are unchanged. Existing unwritten
readers continue to return zeros; existing written readers see data only after
the COW mapping is published. Update the semantic specification and retain
cross-reader fixtures before claiming portable qualification. This is an
experimental writer change within epoch 0; no new C ABI is implied.

Require exact physical reuse for private reservations, fresh storage for shared
or stale-marked ranges, mixed-write byte oracles, signed/maximum offset cases,
zeroed partial-block tails, low-space admission measurements, completed-write
and failed-barrier injection, both-checkpoint and snapshot crash oracles, and
subsequent overwrite/release/reclaim checks. A returned write failure may alter
unwritten physical payload but cannot alter any old logical byte.
