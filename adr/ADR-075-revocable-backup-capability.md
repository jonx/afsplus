# ADR-075: Require revocable host authority for trusted snapshot backup

Status: Accepted

## Context

A consistent backup must include files that were deleted or restricted after
capture. Rechecking current live-file permissions can prevent completing that
backup and cannot resolve deleted objects without additional policy. Conversely,
historical access is powerful and cannot be authorized merely by possession of
a snapshot identifier or an already-open reader handle.

The owner selected a revocable backup capability for the first trusted backup
service, after comparing it with live-file permission checks on every read.

## Decision

Require explicit host-granted backup authority for snapshot management and
historical reads. Scope that authority to the filesystem being accessed. The
host authenticates and grants it; untrusted clients cannot mint authority by
supplying an identifier or a boolean. Do not persist host grants in the volume
or infer them from on-disk ownership metadata.

Check authority on every operation, including enumeration, metadata access,
opening views, reading through existing handles and management mutations.
Revocation denies subsequent operations, even through previously opened handles.
Do not silently substitute current live-file permission evaluation: the trusted
backup authority can read the captured state despite later live restrictions,
renames or deletion. This privilege must be exposed explicitly by an adapter,
never inherited accidentally from ordinary file-read access.

Closing or dropping a handle remains permitted after revocation so resources
and deletion-blocking leases can be released. Revocation cannot retract bytes
already returned. Define the operation-admission boundary and synchronize it
with revocation in each host adapter; tests must cover operations on both sides
of that boundary. An in-flight operation must not grant continuing authority to
subsequent operations or duplicated handles.

Ordinary-user historical browsing, rich ACL evaluation, cross-host identity
mapping and live-file access remain separate Q5 decisions. A backup capability
is not evidence that those policies have been implemented.

## Compatibility and resources

This is a filesystem-neutral host authorization contract. It changes no on-disk
records, feature identities or persisted ACL semantics. API-v2 capability and
error assignments, ABI/versioning, Rust mapping and adapter compatibility must
be specified before adding the interface. A feature capability describing
snapshot support is distinct from a caller's authorization to use it.

Require bounded per-operation authorization and handle cleanup. Measure retained
handles, revocation contention and cancellation in the host implementation;
never use a cached initial authorization check as a permanent reader grant.

## Qualification

Use a filesystem-neutral consumer and authority harness. Cover authorized
capture/enumeration/read/delete, missing or wrong-filesystem authority, revocation
between calls on existing and duplicated handles, and close after revocation.
Denied calls must not expose names, metadata or bytes or mutate the filesystem.
Verify that authorized backup still reads exact captured bytes and metadata
after live permissions change or objects disappear. Re-grant behavior must use
explicit host authority, not resurrect a revoked grant implicitly.

Exercise revocation against concurrent operation admission and preserve the
accepted busy-on-active-reader deletion contract. Real host adapters must prove
their authentication and privilege bridge; a host-side authority harness alone
does not qualify an operating-system security boundary.
