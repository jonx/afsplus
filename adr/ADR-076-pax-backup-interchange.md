# ADR-076: Use PAX tar with versioned backup preservation metadata

Status: Accepted by the owner; archive profile and interoperability qualification required
Amended by: ADR-078, ADR-080, ADR-081, ADR-082

## Context

The consistent-snapshot reader and revocable backup authority provide a source
for a real archive/restore consumer. A dedicated container would permit direct
control of its records but require dedicated tools for ordinary recovery.
The owner selected PAX tar with versioned preservation metadata instead.

The [GNU tar manual](https://www.gnu.org/software/tar/manual/tar.html) describes
PAX-based extended attributes and ACLs, plus GNU sparse extensions. These are
interoperability inputs, not proof that any tar reader preserves every semantic.

## Decision

Use PAX tar as the first backup interchange container. Preserve ordinary paths,
file contents and supported link types using interoperable archive entries.
Add an explicitly versioned preservation profile for metadata that ordinary tar
headers cannot express faithfully. Describe semantic filesystem objects and
relationships, never AFS+ physical blocks, tree encodings or allocator state.

A profile-aware restore tool must preserve all required metadata or refuse the
unsupported state explicitly. It must not silently downgrade security metadata,
original names, timestamps, link relationships or the specified sparse-file
semantics. Unknown required metadata needs lossless opaque transport or explicit
refusal. Do not imply that a generic tar extraction performs complete restoration.

Qualify ordinary-file recovery using independent archive implementations. Sparse
extensions and nonstandard preservation metadata need separate compatibility
oracles; successful listing or extraction of a small regular file is insufficient.
Keep generic recovery useful while making the profile-aware preservation promise
precise and testable.

Distinguish a complete archive from a truncated, canceled or revoked backup.
A syntactically valid tar prefix is not proof of a completed backup. The profile
must define required metadata, content integrity and completion validation before
it can report a complete restoration. Partial salvage is a distinct, explicit
operation with a loss report.

Use the authorized snapshot API as the source and semantic restore operations
as the destination. A changed live namespace must not leak into the archive.
Source or destination paths must not escape the explicitly selected namespace,
and an archive must not gain host authority by containing ownership or security
metadata. Restore privilege is checked by the host.

## Compatibility and qualification

This chooses the archive direction, not exact PAX keyword identities, metadata
encoding or an epoch-1 disk format. Specify the versioned profile and parser
bounds before generating normative fixtures. No filesystem feature bit, C ABI,
legacy DOS interface or on-disk AFS+ record changes follow from this choice.

Require exact archive/restore comparisons for names, links, content, timestamps,
protection, sparse ranges, attributes and security metadata. Distinguish stable
metadata from destination-local object identities and allocation accounting;
define the mapping explicitly instead of copying physical identity.

Cover interrupted output, revoked authority, malformed lengths, duplicate or
conflicting metadata, missing members, unsupported required extensions,
integrity failures and destination isolation. Measure streaming memory, directory
and hard-link bookkeeping, sparse-file amplification and restore publication
costs on the application workloads. Missing provider APIs become implementation
work; they do not justify silently weakening the preservation contract.
