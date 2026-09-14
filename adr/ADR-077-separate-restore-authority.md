# ADR-077: Separate destination restore authority from backup authority

Status: Accepted by the owner; restore interface and host qualification required
Amended by: ADR-083, ADR-089, ADR-092

## Context

Backup needs consistent historical reads and snapshot management. Restoration
additionally writes objects and restores protection, timestamps and eventually
security metadata. Giving every backup job those destination powers would widen
the effects of a faulty or compromised backup process.

The owner selected separate backup and restore grants after comparing a single
combined grant with destination-scoped authority issued for a restore job.

## Decision

Require a separate revocable host-granted restore authority for the selected
destination. A backup grant must not authorize file writes or metadata restore;
a restore grant must not implicitly authorize historical source reads. A job
that needs both receives both explicitly from the trusted host.

The host authenticates the restore request and chooses its destination scope.
Archive paths, object identities and stored security metadata cannot expand
that scope or mint authority. Expose destination operations through a checked
consumer interface; privileged backend hooks remain with the host.

Check restore authority on every operation, including through existing handles.
Define operation admission and revocation using the same exclusion contract as
ADR-075: already admitted work may finish, revocation drains it, and subsequent
operations are denied. Revocation cannot undo already committed writes. Closing
handles and releasing resources remain possible after revocation.

A revoked, interrupted or failed restore must not be reported as complete.
Distinguish an intact completed destination from partial work and preserve the
ADR-076 profile's explicit integrity and preservation requirements. Authorization
does not remove the need for destination isolation, unsupported-metadata refusal
or crash-safe publication.

## Compatibility and qualification

This changes host authority, not filesystem disk records. The additive restore
API must define destination scope, handles, existing-destination behavior,
versioning and denial mapping before it is advertised through a native bridge.
No C ABI, permission-bit reinterpretation or rich ACL evaluation is implied.

Require tests rejecting backup-only grants on restore operations and restore-only
grants on historical reads. Cover wrong destinations, revoked grants, existing
and duplicated handles, cleanup, in-flight admission, and no backend writes or
caller-visible data leakage on denial. Use filesystem-neutral provider tests and
real AFS+ metadata/content restoration, then qualify native host authentication
and path confinement independently.
