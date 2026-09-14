# ADR-083: Stage opaque metadata before destination publication

Status: Accepted
Amended by: ADR-084
Amends: ADR-077, ADR-082

## Context

Opaque snapshot metadata can be enumerated and read in bounded chunks. Installing
those chunks directly into active destination security state would expose
partial descriptors after interruption or revocation. Whole-value buffering
would impose memory proportional to the descriptor size on constrained hosts.

## Decision

Use a staged opaque-metadata upload under the destination object's original
restore grant. The host explicitly enables a maximum value size. Each upload
uses a separate handle-budget unit and retains the destination object lease.
The consumer cannot manufacture an upload, change its target, clone its progress
or replace its grant. Attribute and security channels remain distinct.

Stream sequential chunks into provider-private staging. Refuse excess bytes
before provider invocation. An uncertain staging-write error permanently fails
that upload. Finish consumes the handle and requires exactly the declared byte
count plus current grant admission. Only the provider's atomic finish installs
the complete value with its exact key and encoding. Unknown formats require
lossless support or explicit refusal, never translation by assumption.

Dropping an unfinished upload releases provider-owned staging without requiring
a live grant. Providers own cleanup through their upload value's destructor;
cleanup must not modify active metadata. Publication errors may leave the old
state or complete new state, never a partial descriptor, and must not report
success. Providers must reconcile uncertain publication before further mutations.
A returned success means the provider's qualified atomic publication contract;
filesystem synchronization and whole-restore completion have separate gates.

Use an optional Rust provider extension with an associated staging type, keeping
ordinary restore providers source-compatible. The AFS+ provider explicitly
refuses the extension until its storage/publication implementation is qualified.
This specifies a semantic API, not an AFS+ disk representation or ACL evaluator.

## Compatibility and qualification

No C ABI, legacy DOS interface, filesystem feature identity or disk record changes.
The host controls per-value admission and simultaneous handle counts. Providers
must stream staging without requiring whole-value RAM; a host prototype may use
memory only within its declared test envelope, without native resource claims.

Require exact unknown-binary installation, empty values, no pre-finish visibility,
abort cleanup, premature/excessive lengths, write/finish faults, revoked and
foreign authority, lease retention and budget recovery. Verify admission held
through backend calls. Actual durable providers require crash/cleanup oracles
and measured constrained-host resource bounds before advertising the capability.
