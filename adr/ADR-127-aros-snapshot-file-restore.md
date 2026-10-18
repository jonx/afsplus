# ADR-127: Restore a snapshot file into an explicit fresh destination

Status: Accepted
Amended by: ADR-133
Amends: ADR-071, ADR-077, ADR-119, ADR-125

## Context

A local snapshot supplies consistent backup state. Restoring its data by
sharing historical physical blocks with live storage requires a rule for
ownership gaps under ADR-071. An explicit destination also gives a bounded
consumer for the existing metadata restoration contract.

## Decision

The local file restoration operation reads a pinned snapshot and writes fresh
live allocations. It preserves captured file bytes, sparse allocation shape,
comment, protection, owner and timestamps, and supported opaque metadata.
Unsupported captured metadata refuses the operation before target mutation.

The caller supplies a live destination-directory lock and one target name.
The adapter confines restoration to that directory. An existing target is
refused. Build and verify a staged file in the destination directory, then
publish it with a non-replacing rename that preserves the captured file's
change timestamp. Parent-directory timestamps describe the live publication.
Failure before publication leaves the requested target absent; staged-file
cleanup failures are reported. An uncertain checkpoint publication is an
explicit error and must be inspected before retry.

Historical read authority and restoration authority are distinct host checks.
A packet restore requires both at admission. The packet configuration supplies
separate callbacks. The native handler applies BACKUP and RESTORE mount
policies independently. Each defaults to DENY. ROOT queries security.library
and admits the current sender's root UID. SINGLEUSER additionally permits
admission without that library in an explicitly configured administrator
trust domain. The sender's reply port must be a signal port with a task.
No request boolean grants authority. Close remains permitted after revocation.

Local backup captures a registered volume snapshot. LIST and DELETE use the
same historical authority. Restoring a regular file names its snapshot, source
path and explicit new destination. Directory restoration, external archive
transport and boot-root selection have their own qualification boundaries.
The command reports the implemented boundary and errors without weakening
metadata silently.

## Compatibility and resource consequences

Add one semantic extension operation, SNAPSHOT_RESTORE_FILE, and expose a
filesystem-neutral fresh-allocation restoration method. No disk layout or
feature bit changes. Admission budgets bound page and copy work; copied data
costs fresh space proportional to the source allocation plus transaction
metadata. Restoring does not reclaim or retarget retained snapshots.

## Qualification

Verify complete captured bytes and metadata after publication and remount,
unchanged snapshot bytes, fresh allocation ownership, holes and unwritten
storage, busy deletion, existing-target refusal and rejected opaque metadata.
Inject failures before data, metadata and publication. Test distinct backup
and restore authorization, revocation between packets, confined destination
names and same-volume lock validation. Retain host and VM artifacts separately
from installed-generation native acceptance.
