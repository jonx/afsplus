# ADR-126: AROS owner identifiers preserve the disk values

Status: Accepted

Related: ADR-031, ADR-101, ADR-118, ADR-125

## Context

The object record stores 32-bit owner UID and GID. AROS Examine and ExAll
carry 16-bit owner fields, and ACTION_SET_OWNER packs two 16-bit values.
AROS security.library assigns identifier 0 to nobody and 65535 to root.
Saturating a larger disk identifier to 65535 would attribute an unmapped
owner to the host administrator.

## Decision

The AROS adapter treats representable owner identifiers as AROS account
identifiers. Values from 0 through 65535 are reported unchanged. Disk values
above 65535 are projected to AROS nobody, identifier 0, in the classic DOS
fields. The projection changes only the query result. It never changes the
object record. The full-width EXT_STAT result reports both stored 32-bit
values without projection.

ACTION_SET_OWNER takes the UID from the high 16 bits of dp_Arg4 and the GID
from the low 16 bits, following DOS SetOwner. Both stored identifiers change
in one metadata operation. Existing ACLs, security descriptors and protection
bits remain intact. Read-only mounts reject the mutation. The single-user
adapter applies host administrative policy; this operation does not add a
multi-user authorization evaluator.

Cross-host account database mapping is host policy. A numeric identifier
from a POSIX account database gains no universal principal identity merely
by being stored in an object record. In particular POSIX UID 0 and AROS UID
65535 require an account mapping when transporting ownership between hosts.

## Boundary and resource impact

AfsplusArosFileInfo appends two 32-bit fields, increasing its size from 64 to
72 bytes. AfsplusArosStat appends the same fields, increasing its size from
88 to 96 bytes. The handler and static library are rebuilt together under
boundary ABI version 2. The EXT_STAT sized-output contract preserves its
88-byte minimum and writes only the caller's advertised capacity.

The change adds eight bytes to each pending Examine/ExAll entry and no extra
block reads to those queries. The on-disk codec, allocation model, repair
rules and feature identities are unchanged. Existing conformance images are
readable because only the host projection and C boundary change.

## Verification contract

Tests retain identifiers above 65535 through a flush and remount, expose
them through full-width metadata, and project them to nobody in DOS fields.
Packet tests verify ACTION_SET_OWNER path resolution and the packed UID:GID
order. Zero is tested as a supplied value rather than an omitted update.
