# ADR-092: Reopen created restore entries within the isolated destination

Status: Accepted under the owner's delegated recommended-option authority; namespace consumer and native qualification required
Amended by: ADR-093
Amends: ADR-077, ADR-091

## Context

A restore consumer must revisit primary files for hard links and finalize metadata
after namespace edits. Retaining every object handle makes active-resource usage
proportional to object count. The destination starts as a selected empty directory
and is exclusively owned by the trusted provider for the restore service lifetime.

## Decision

Add an optional `lookup_created(parent, name)` operation to the restore backend,
service and checked consumer facade. It resolves exactly one ordinary name within
an already scoped directory, returns an object handle carrying that directory
handle's original grant, and consumes the usual active-handle budget. It does not
follow symlinks or accept paths, parent traversal, raw object IDs or external
object import. Missing provider support explicitly refuses the operation.

Hold the grant admission permit across parent-kind validation, provider lookup
and handle construction. Validate the component and reserve a handle slot before
provider calls. Reject non-directory parents. A failed lookup releases its slot;
closing a returned handle releases it independently of the parent. Check the
original grant on every later operation. A separately issued valid grant may
obtain a new root and reopen the same in-scope entry; it never reactivates an old
handle or changes that old handle's original grant.

The host must provide an initially empty, isolated destination and exclusive
namespace ownership for the service lifetime. No consumer rename, delete,
external insertion or symlink traversal can move an object across the scope.
The AFS+ provider owns its Volume and resolves directory entries without following
targets. This is access to entries created inside the isolated restore tree,
not existing-destination merge/overwrite, persistent resume or general file lookup.
Those contracts retain their own design and qualification gates.

The consumer may walk directories with two active handles, dropping each previous
parent after the next handle is obtained. Linking a retained file while walking
a destination path needs an additional slot. Reopening trades lookup work for
bounded active handles; archive namespace indexes, paths and retained metadata
need separate resource limits or spooling. The operation writes no filesystem
state and introduces no filesystem record, feature bit or C ABI change.

## Alternatives and qualification

Keeping all handles was rejected as the only consumer strategy because large
object counts would dictate active resource usage. Exposing raw object IDs or
unconfined filesystem paths was rejected because archive input could expand the
destination authority. Requiring a complete persistent-resume design first was
rejected because reopening within a live, exclusively owned fresh destination
has a narrower lifetime and a directly testable scope invariant.

Require two-slot traversal with closed/reopened handles, original-grant revocation,
new-grant independence, foreign-service refusal, slot release after failures,
invalid names and non-directory/symlink parent refusal. Hold the operation permit
during all provider calls. Use real AFS+ nested directories, hard-link identity,
finalized metadata, an untouched outside object and remount oracles, and measure
zero lookup writes/barriers. Native providers must independently establish
isolation and no-follow semantics before advertising the extension.
