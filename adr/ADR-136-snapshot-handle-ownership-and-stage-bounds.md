# ADR-136: Bind snapshot handles to their opener and keep captured stages private

Status: Accepted
Amends: ADR-125, ADR-127, ADR-128, ADR-130, ADR-133

## Context

Snapshot reader and snapshot directory identifiers come from one sequential
counter, so they are easy to guess. Only admission checked authority, and close
needed none, so any task could close, read through or advance another task's
reader. Under the ANYONE policy of ADR-135 every local task is admitted.

A snapshot taken while a failed restore had left a private stage captured that
stage. The snapshot listed it, and restoring its parent published the
unfinished copy as an ordinary object.

Tree restoration accepted any depth, while reclamation reaches at most 64
levels below its scope and cannot start inside a stage. A deeper stage left by a
failure could never be reclaimed. A captured attribute set close to the 64 KiB
limit left no room for the stage marker and failed after staging began.

## Decision

A snapshot reader or snapshot directory belongs to the task that opened it.
Lookup, stat, read, directory open and read, file and tree restoration, and
close each refuse any other task with ERROR_INVALID_LOCK. The owner can still
close after its authority is revoked. Exhausting the reader or directory budget
reports ERROR_NO_FREE_STORE.

A captured object that was a private stage when the snapshot was taken stays
private in that snapshot. Listings omit it, and lookup, stat, read and
restoration answer not found, as the live volume did. A marker the view cannot
decode hides the object too.

Tree restoration refuses a depth budget above 64 levels before writing. A
captured attribute set that leaves no room for the marker is refused before
anything is staged. Reclamation blocks an entry whose marker cannot be read,
and continues with its siblings.

Snapshot directory records carry UTF-8 names whatever the mount encoding, as
the live directory read does.

AFSPlusReclaim restarts a fresh scan when its cursor goes stale, at most eight
times, and then reports that the subtree kept changing.

## Consequences

A task that ends without closing its readers keeps their snapshots pinned until
the volume is unmounted. That was already the case; ownership only stops other
tasks from using those readers.

## Qualification

The packet matrix refuses a read and a close from a second task and accepts
both from the opener. Rust FFI tests do the same for stat, read and close. VFS
regressions cover a captured stage, which is neither listed, read nor restored,
a 65-level budget and a full attribute set; all three fail on the previous code.
