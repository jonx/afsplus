# ADR-125: Admit historical operations through the AROS host authority

Status: Accepted
Amended by: ADR-127, ADR-134, ADR-135, ADR-136
Amends: ADR-075

## Context

The AROS handler transports management requests through shared-memory DOS
packets. Historical reads require the host authority of ADR-075. Persistent
snapshot identifiers and runtime reader identifiers identify resources only.
AROS security.library uses UID 65535 for root and UID zero for nobody.

## Decision

The packet adapter accepts a host authorization callback. It invokes that
callback at admission to every snapshot operation, including reads through an
existing reader. The callback receives the actual DOS packet, including its
reply task. An absent callback denies admission. Closing readers remains
permitted after authorization is revoked.

The native handler queries security.library's current owner for the sender's
reply task at every admission. Only the root UID admits historical access.
Unknown senders and nobody are denied. The handler serializes operations; a
change to ownership before a subsequent admission revokes that operation.
An admitted operation may finish and return its bytes, as ADR-075 permits.

A host administrator may select BACKUP=SINGLEUSER in the DOSDriver Control
string on an installation without security.library. This declares the shared
single-administrator trust domain and admits its clients. When security.library
is available, its root check applies regardless of that choice. The default
BACKUP=DENY disables historical access. Clients cannot enable the mount policy
with a request field. A Control string is administrative configuration.

AROS shared memory permits tasks to corrupt message structures and kernel
state. This adapter provides the operating system's task-owner admission
contract; it does not provide isolation against arbitrary memory corruption.
Snapshot capabilities report supported semantics separately from authority.

Reader identifiers are scoped to the mount and bounded by the configured
reader budget. Opening a reader pins its snapshot; explicit close releases
that lease. Deletion of an open snapshot reports busy. Teardown releases all
leases. There is no implicit expiry or deferred deletion.

## API and resource consequences

Extend the native packet configuration with the host callback and increment
its ABI. The filesystem-neutral VFS exposes opaque reader identifiers and
bounded list and directory pages. The C interface reports snapshot support
as an entry-point group; absent volume support reports not implemented.
Mount budgets are explicit runtime inputs. These changes alter no disk bytes.
Snapshot creation first publishes pending VFS work, then publishes the
snapshot through the common durable checkpoint. Restore copies into newly
allocated storage under the existing restore contract; live-root rollback
requires a separate decision.

## Qualification

Test denied capture, enumeration and historical reads without disclosing
bytes, current root admission, non-root and unknown sender refusal, revocation
between calls, close after revocation, reader-budget exhaustion and busy
deletion. Host packet shims establish dispatch and callback behavior; a VM
probe establishes the actual security.library and DOS reply-task bridge.
Native qualification uses an installed generation and retained harness runs.
