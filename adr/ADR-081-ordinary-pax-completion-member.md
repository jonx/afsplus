# ADR-081: Use an ordinary terminal member in a separate archive namespace

Status: Accepted under the owner's delegated recommended-option authority; preservation-profile integration required
Supersedes: ADR-080

## Context

The version-1 prototype's terminal PAX global header failed independent recovery
with Apple Python 3.9.6 tarfile: its reader requires a following member after a
PAX header and reports a missing subsequent header at the end markers. The
independent SHA-512/256 check passed once OpenSSL 3 supplied the algorithm absent
from the bundled Python/LibreSSL hash provider. The framing choice must change
to preserve ordinary recovery with that existing reader.

## Decision

Use envelope version 2. Preserve ADR-080's SHA-512/256 digest domain, counters,
limits and receipt boundary, with these archive-structure changes:

- The beginning PAX global header is named `_AROS_BACKUP/begin` and identifies
  envelope version `2` and algorithm `sha512-256`.
- The terminal control is an ordinary regular file named
  `_AROS_BACKUP/complete.pax`, containing the unique PAX control records. Its
  end marker is `2`, mode is `0600`, and other ownership/time fields are zero.
- Source namespace objects occupy `files/`, with an optional `files` directory
  representing the captured root. Source names that resemble control names are
  preserved underneath this subtree. Auxiliary preservation records may occupy
  `_AROS_BACKUP/metadata/`.
- Envelope body global headers are refused. Local PAX headers are permitted;
  the preservation layer validates their resolved paths and metadata before
  issuing a preservation outcome. Resolved names must honor the same namespace
  separation. Hard-link targets refer to the source-data subtree. Symlink data
  never authorizes a restore job to follow a link outside its destination.

Ordinary tar extraction produces a `files` subtree plus auxiliary metadata.
Profile-aware restoration unwraps the source root and consumes auxiliary data
without inserting it into the destination namespace. This gives arbitrary
source names an unambiguous representation and gives large metadata a streaming
container. Generic file recovery does not imply full metadata preservation.

## Compatibility and qualification

Version 1 is a superseded local prototype; its accepted decision remains as
history. Reject it explicitly. No AFS+ filesystem format or ABI changes follow.
The detailed contract is [the envelope specification](../spec/backup-envelope.md).

Require independent Python and bsdtar recovery of version-2 body files,
independent digest/count verification, reserved-identity and path traversal
refusal, and all truncation/mutation/receipt tests from ADR-080. Match default
and compact software hash backends. Full profile/path-override interpretation
and actual authorized snapshot/restore consumers have separate gates.
