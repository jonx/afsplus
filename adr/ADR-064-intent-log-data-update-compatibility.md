# ADR-064: Fail closed on intent-log data-update records

Status: Accepted for the experimental version-3 record set; final wire freeze remains M14
Amends: ADR-063

## Context

ADR-063 requires an implementation that cannot validate and replay the active
intent-log record version to fail closed. Record version 2 contains only
create, delete and rename operations. Version 3 adds writes and truncates of
existing files and can therefore name replacement COW data blocks that are
FREE in the base checkpoint's allocation bitmap until replay.

Merely incrementing the record version is insufficient. The version-2 scanner
deliberately treats an undecodable slot like an empty, stale or torn log tail.
It would therefore ignore a valid version-3 record and could mount the base
checkpoint read-write after an acknowledged fsync. Reusing the existing base
intent-log feature bit cannot distinguish that unsafe reader from a version-3
reader.

The format is still experimental, but silent durability loss is not an
acceptable prototype compatibility behavior.

## Decision

1. Allocate `INCOMPAT` bit 1 to the permanent identity
   `org.aros.afsplus:intent-log-data-updates`.
2. The feature depends on `INCOMPAT` bit 0,
   `org.aros.afsplus:intent-log`. Identification validation rejects bit 1
   without bit 0.
3. New formatted volumes with an intent-log area enable both bits. A binary
   that knows only the version-2 namespace log consequently rejects the
   volume during feature negotiation, before scanning or writing it.
4. Existing prototype volumes carrying only bit 0 remain readable. They may
   emit and replay namespace-only version-2 records, but the existing-file
   write/truncate window API returns `FeatureDisabled`.
5. Namespace-only groups continue to encode record version 2. A group that
   contains an existing-file write or truncate encodes version 3.
6. A version-3 record found without bit 1 is corruption. It is never treated
   as an ordinary torn tail.

The numeric assignment and record version remain experimental wire values
until M14, but the feature identity itself is never reused.

## Validation

- identification codec tests enforce the bit-1 dependency;
- codec tests prove namespace-only groups remain version 2, data-update groups
  use version 3, and historical version-2 records still decode;
- an old-profile image with only bit 0 rejects data updates while retaining
  namespace replay;
- the checker and mount scanner reject data-update records whose feature is
  absent.

## Consequences

The extra bit deliberately makes new log-enabled prototype images unavailable
to older writers even when they currently contain only namespace records. The
cost is conservative and visible; the alternative can acknowledge a write
and later discard it silently.

Portable C and other independent readers must negotiate both identities
before claiming support for version-3 log replay.
