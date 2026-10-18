# ADR-130: Guard restore stages through bounded per-object metadata

Status: Accepted
Amended by: ADR-133
Amends: ADR-129

## Decision

Use the existing opaque internal attribute `system.restore.stage` on every
staged object. Its eight little-endian bytes identify the stage root. Mark the
root with its own identifier. Mark each descendant before copying historical
bytes or metadata into it. An ordinary VFS operation reads the object's marker
and, when present, the root marker. It denies access while the root retains
its self marker, including direct object-ID access. Malformed markers fail
closed. This needs no ancestor search, mount inventory or full namespace scan.

Final non-replacing publication removes the self marker in the same transaction
as the root's namespace move. Child markers then become inactive derived
metadata. Object identifiers are monotonic and never reused. Removal of an
inactive root also leaves the derived markers inactive. Child publication
inside a staging tree retains its marker. Independent authorized core access
performs copying, verification and cleanup, without entering ordinary VFS paths.

The marker is managed metadata: ordinary attribute reads and enumeration omit
it, writes or removals refuse it, and source restoration excludes it. Public
file and tree attributes and metadata remain exactly those captured. The
physical derived marker remains stored on published descendants; its owned
attribute-chain space is included in restoration overhead. This is an internal
attribute contract on the existing disk format, not a format version change.

Ordinary stat/open/read/write, attribute and metadata edits, clones, links,
namespace edits and enumeration apply the guard. A directory page skips
marked entries and continues its raw cursor, so an unrelated live root remains
listable. The trusted core can inspect the raw managed metadata. No filename
is reserved and no shared-memory isolation or untrusted-core claim is made.

## Qualification

Retain standalone and tree stages through copy and cleanup failures, remount,
and refuse direct-ID and normal DOS access or mutation while keeping root
listing usable. Refuse clearing or forging managed markers. Check bounded
object/root marker reads, no namespace inventory, and final captured public
attributes and metadata. Enumerate publication failures and preserve the
historical source. Retain original exposing code as a negative control.
