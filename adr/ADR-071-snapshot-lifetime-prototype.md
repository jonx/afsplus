# ADR-071: Prototype persistent lifetime accounting and busy snapshot deletion

Status: Accepted for the integrated experiment; shipping wire and resource qualification open
Amends: ADR-070
Amended by: ADR-072, ADR-074

## Context

The owner approved the lifetime-ledger and bounded persistent reclaim-scan
candidate in proposal S5. The isolated accounting experiment demonstrates
unbounded unrelated retention under an oldest-generation FIFO barrier.
A snapshot needing one old data block fills a 64-block model after 62 temporary
create/delete cycles. Lifetime intersection plus traversal past protected
entries completes the full workload with one retained block.

The owner also selected refusal of snapshot deletion while readers have active
handles. These decisions govern the next integrated prototype.

## Decision

Track allocation lifetimes for snapshot-reachable physical runs. Preserve birth
across run splitting and reflinks; set retirement when the last live reference
disappears. For a retired run, a retained generation S intersects its lifetime
when `birth <= S < retirement`. A run can leave retained ownership only when
no retained view intersects that lifetime.

Use bounded persistent traversal that advances past protected entries. Transfer
eligible runs to ordinary checkpoint quarantine atomically with their removal
from retained ownership. Keep the existing selectable-checkpoint delay and
bitmap authority; transferred storage cannot be queued twice or reused early.
Persist progress so reclamation resumes across remount and interrupted recovery.

Classify namespace data and metadata explicitly. Allocation/reclaim machinery,
registry nodes and lifetime-ledger nodes are housekeeping: their protection
follows selectable checkpoints and their allocations never recursively create
ledger entries. The checker must validate that separation and account for all
physical blocks, including snapshot-only and orphan storage.

Force data COW while any snapshot is registered. In-place opt-in cannot weaken
snapshot immutability. Snapshot creation resolves pending log/window work first
and acknowledges only after the common durable checkpoint publication.

Deletion returns busy while any runtime reader holds a handle for that snapshot.
Closing the last handle permits explicit deletion. Runtime handles disappear
on reboot; registered snapshots persist. No deferred-deletion tombstone is
introduced by this prototype. Never expire a registered view implicitly because
space is low.

## Integration and compatibility gates

The follow-up format record defines tree identities, registry/ledger records,
root negotiation, ID exhaustion, cursor and independent parser rules before
those semantics enter the writer. Feature-absent images retain their declared
layout. An unaware reader must reject snapshot-enabled images before writes.

Keep normal mount bounded. Qualify large retained sets, maximum configured view
counts, scan wrap, low-space admission and release, last-reference transitions,
metadata amplification and every modeled create/delete/reclaim publication cut.
Read complete retained namespaces and bytes after remount using independent
oracles. Include Rust/C conformance or explicit feature rejection, checker
corruption cases, resource measurements and repair behavior.

The model does not establish integrated I/O, memory or crash costs. Admission
limits and reserves require those measurements. Snapshot-to-live cloning and
rollback need an explicit lifetime rule before introduction; a physical block
must not acquire a fabricated continuous lifetime across an ownership gap.
