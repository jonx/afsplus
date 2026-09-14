# ADR-095: Bind symlink archive targets to exact scoped restoration

Status: Accepted
Amends: ADR-093, ADR-094

## Decision

A symlink group begins with `symlink-v1-{full|recovery}-N.pax` in the metadata
namespace. Its payload is the existing exact object descriptor with symlink kind.
The following zero-payload ordinary symlink consumes N+1 and carries path, mtime
and exact opaque target in a local PAX linkpath record. Header names and neutral
fields follow the namespace-group contract. Full groups append a complete opaque
inventory at N+2; recovery omits it with explicit captured knowledge.

Read the target through the original captured grant, bound its required length
by the caller's PAX record-byte budget, validate nonempty NUL-free UTF-8 and
agreement with captured logical size, and recheck bytes/stat/knowledge after
emission. Do not normalize or resolve the target. Errors poison archive completion.

Restore only verified replay into a freshly created scoped symlink. Validate the
profile, kind, canonical source path, destination basename, ordinal/header binding
and target before creation. Full restore refuses recovery groups; opaque inventories
follow directory full/recovery rules. Apply exact core metadata and compare kind,
size, one-link identity semantics, timestamps, protection and exact target readback.
Return the created handle and explicit opaque preservation/loss report. Resource
refusal never changes the requested preservation mode. A partial destination may
survive a later error, which prevents group success and poisons further replay.

The target is authoritative in its bound ordinary member; no second independently
interpreted target field is introduced. The integrity envelope protects its bytes.
The enclosing job still owns parent placement, duplicate/omission detection,
complete enumeration, final metadata, durable losses, EOF and synchronization.
A target containing parent or absolute syntax is preserved as text; subsequent
namespace creation cannot use a symlink as a directory. Generic tar extraction
requires its own host path-safety policy and is not a full preservation claim.

## Qualification

Require real captured target export after live unlink, exact scoped restore and
remount, independent tar target inspection, full/recovery profile refusal and
opaque inventory oracles, malformed binding/target cases, short target limits,
revocation and failures preventing completion. Native and whole-job qualification
remain separate gates. No filesystem disk layout or C capability identity changes.
