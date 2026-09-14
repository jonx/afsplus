# ADR-091: Bind regular-file metadata, allocation and opaque inventory groups

Status: Accepted under the owner's delegated recommended-option authority; namespace/job and native qualification required
Amended by: ADR-092
Amends: ADR-078, ADR-082, ADR-084, ADR-086, ADR-090

## Context

Separate component reports cannot prove that exact object metadata, sparse
contents, reservations and opaque inventories belong to one restored file.
Content recovery also needs to consume a full archive on a provider without
opaque metadata support, without treating omitted or uninspected values as empty.

## Decision

Use an explicit versioned regular-file group. Its first ordinary auxiliary
member is `file-v1-full-N.pax` or `file-v1-recovery-N.pax` inside the metadata
namespace and carries the existing version-1 object metadata payload. Follow
with allocation ordinal N+1 and sparse ordinal N+2. A full group also carries
an inventory manifest at N+3 and its consecutive values. A recovery group omits
opaque inventories while retaining their captured knowledge in object metadata.
This profile distinction is carried by the archive, independently of the
requested restoration mode; no error silently switches either mode.

Bind exact path, regular-file kind and modification timestamp across the object
and sparse member before data writes. Export checks captured stat and inventory
knowledge at both ends. Full export requires inspected inventories. Full restore
refuses recovery groups before writes. Recovery may consume either group;
validate and drain full-group inventory manifests/values without requiring
opaque destination support, and return their exact transported counts/bytes.
Always report omitted Present or Uninspected knowledge explicitly. Reservation
losses follow ADR-090. Metadata core fields are restored in either mode; inability
to represent protection/timestamps refuses the operation rather than truncating.

Apply core metadata after content and opaque operations. Read back kind, logical
size, protection and all three timestamps and compare exact values before a file
report. Keep grant checks, bounded pages/buffers/record storage, ordinal admission
and sticky error behavior. Earlier committed changes can survive errors; this is
not atomic whole-job publication. Empty inventories need no opaque upload.

The group describes one primary regular file. Directories, hard-link graph and
alias agreement, symlinks, complete namespace enumeration, persistent job loss
reports, archive EOF and destination synchronization are enclosing-job gates.
A successful file report does not claim that those gates passed. No filesystem
record, feature bit or C ABI is changed. Group details belong in
[regular-file groups](../spec/backup-file.md).

## Alternatives and qualification

Implicitly inferring omitted inventories from EOF was rejected because malformed
or truncated preservation archives could then masquerade as intentional recovery.
Expanding an unknown inventory into an empty manifest was rejected because it
asserts knowledge the provider lacks. Keep the existing exact metadata payload
rather than replacing its accepted field meanings.

Require real captured AFS+ recovery and remount metadata oracles, full-group
fixtures with opaque values, recovery from full groups without uploads, mismatched
path/kind/mtime refusal before writes, inventory conflicts, source mutation and
revocation, destination rounding/missing support, ordinal/resource limits and
partial-outcome poisoning. Small-buffer fixtures do not establish actual native
or older-system peak-memory and durability qualification.
