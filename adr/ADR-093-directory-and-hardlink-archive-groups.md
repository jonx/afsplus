# ADR-093: Bind directory and hard-link archive groups to scoped restoration

Status: Accepted under the owner's delegated recommended-option authority; enclosing namespace/job qualification required
Amends: ADR-082, ADR-091, ADR-092

## Context

Regular-file groups do not represent directories or aliases. Recreating an alias
as a second file would lose identity and shared modifications. Applying directory
metadata before creating children also cannot establish final timestamp fidelity.

## Decision

Add versioned directory and hard-link groups using the existing exact object
metadata payload. Their initial auxiliary names are
`directory-v1-{full|recovery}-N.pax` and
`hardlink-v1-{full|recovery}-N.pax` in the metadata namespace. The following source
member consumes N+1 and uses ordinary PAX path/mtime and, for an alias, linkpath.
Directory source members have mode 0700 for generic private recovery; aliases
have mode 0600. Both have zero stored payload. Full directories append a complete
inventory at N+2. Recovery directories omit inventories with explicit knowledge
loss. Aliases do not duplicate contents, allocation or opaque inventories.

Validate exact group/profile/kind/path/mtime/ordinal binding and canonical target
names. Full export requires inspected inventories; full directory import refuses
a recovery group. Full-directory inventories can be validated and discarded in
explicit recovery. Apply core metadata and compare exact readback in either mode.
Directory restoration requires an empty scoped directory. Add an optional bounded
`directory_empty` query under the same grant, kind and no-write contract; the AFS+
provider checks a one-entry page. No missing provider API implies emptiness.

For aliases, the enclosing namespace plan supplies a previously restored primary
handle, its canonical source path and its recorded inventory knowledge. Validate
the alias descriptor against the primary's exact protection/timestamps and that
knowledge. Preflight the destination name and lookup capacity before link creation.
Then create without overwrite, reopen the alias, restore core metadata and verify
identical object identity, size, allocation and incremented link count. Both
primary and parent authority checks apply through the existing restore operations.
Do not infer complete primary preservation or graph completeness from an alias
component report. Full import refuses a recovery alias profile.

Source export rechecks captured stat and knowledge at the end. Failure poisons
further archive use; prior committed destination changes can remain partial.
Ordinal, record, inventory, buffer and active-handle limits apply. The ordinary
source member uses local PAX fields with bounded synthetic raw names, so long
canonical names do not depend on raw ustar field capacity. Raw and effective
auxiliary identities must match their expected ordinals.

Enclosing orchestration owns parent placement, complete enumeration, canonical
primary selection, alias graph consistency, metadata finalization after all
namespace edits, persistent loss reports, EOF and synchronization. Symlinks and
AFS+ opaque storage retain separate implementation gates. No filesystem disk
record, feature identity or C ABI changes follow.

## Alternatives and qualification

Duplicating primary data for each alias was rejected because it changes identity.
Implicitly omitting directories was rejected because empty directories and their
metadata would disappear. Treating successful creation as final metadata fidelity
was rejected because later children/aliases can change timestamps.

Require real captured AFS+ directory/file/alias recovery through one archive,
closed/reopened scoped handles, exact remount metadata and shared identity, and
an untouched outside file. Cover full-directory opaque transport with a semantic
provider, explicit recovery loss, malformed bindings/profiles/targets, primary
metadata disagreement, occupied names, insufficient handle slots, unsupported
emptiness checks and revoked grants. Independent ordinary tar inspection must
recognize directory and hard-link source members. Native and whole-job evidence
must be qualified separately.
