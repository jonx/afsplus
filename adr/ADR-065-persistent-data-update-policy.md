# ADR-065: Persistent per-file data-update policy encoding

Status: Accepted; wire format experimental until M14
Amends: ADR-062

## Context

ADR-062 fixed the semantics of the explicit hybrid data-update policy —
full data COW by default, an explicit per-file opt-in to conservative
private in-place updates — but deliberately assigned no on-disk encoding:
the runtime `DataUpdatePolicy` switch is qualification machinery that
resets to full COW at every mount. ADR-062 requires the eventual encoding
to live somewhere unknown writers preserve, or in a compatibility class
that prevents such writers from silently dropping the choice.

Two properties of the existing format decide the design space:

1. Object-record flags are a **validated namespace**. Every current
   implementation — the Rust core, the checker and the portable C reader —
   rejects an object record carrying an unknown flag
   (`object has unsupported flags`). A choice stored in an object flag
   therefore cannot be silently dropped: a writer either understands the
   flag or cannot read the record at all.
2. The identification block is written once by mkfs and never rewritten.
   A volume-level feature bit therefore cannot be set lazily on first use;
   like `RO_COMPAT` shared-extents (ADR-061), a feature must be activated
   by the formatting profile.

## Decision

1. Allocate object-record flag bit 1, `OBJECT_FLAG_DATA_IN_PLACE`
   (wire `1 << 1` in the 16-bit object flags field), meaning: this file is
   persistently opted into ADR-062 private in-place updates. The flag is
   valid on file objects only; on any other object type it is corruption.
2. Allocate `COMPAT` bit 0 to the permanent identity
   `org.aros.afsplus:data-policy`. A volume whose identification carries
   the bit may hold flagged object records; mkfs activates it per profile.
   The flag on any object of a volume without the bit is corruption, and
   the policy API fails with the feature error on such a volume — the same
   congruence discipline as the shared-extent marker.
3. The filesystem-neutral policy API sets and clears the flag through a
   normal metadata-COW object update (a metadata change: the change
   timestamp advances). Setting requires a file object and the volume
   feature; clearing returns the file to full data COW and is always safe.
   The write path treats a file as in-place eligible when its record
   carries the flag; every ADR-062 eligibility rule (non-extending,
   materialized, mapped, proven private, fail closed on any uncertainty)
   is unchanged. The volume-wide runtime switch remains qualification
   machinery layered on top and still resets at mount.
4. Reading the flag needs no feature knowledge beyond decoding: readers
   that validate object flags already fail closed on it, and readers aware
   of this ADR treat it as informational. In-place updates change
   user-data versioning only; checkpoint transactions, allocation and all
   other metadata stay COW (ADR-062 rule 6).

## Compatibility classification

`COMPAT` is correct for the volume bit: an implementation that ignores it
and always performs full data COW provides strictly stronger durability
with identical visible contents (ADR-062's compatible fallback). The
per-record protection against silently dropping the owner's choice does
not come from the volume bit but from the validated flags namespace: no
existing implementation can rewrite an object record whose flags it does
not understand. The classification prevents the one remaining silent
path — a writer that has never heard of object flag bit 1 cannot mount a
volume, believe every flag it sees is known, and strip the bit, because
bit 1 is only legal on volumes that declare the feature and such a writer
still refuses the flagged record itself.

The encoding remains experimental until the M14 freeze, like every other
epoch-1 surface; ADR-062's remaining gates (exact-generation API tests,
low-space and real-storage qualification, portable-C fail-closed parity)
are unchanged by this ADR and still block the freeze.

## Rejected alternatives

- **An extended-attribute or side-table store** is rejected: no such
  mechanism exists in the prototype format, and inventing one for a single
  bit multiplies the M14 surface without adding protection beyond what the
  validated flags namespace already gives.
- **A lazily-set volume feature bit** is rejected: the identification
  block is immutable after mkfs, and adding runtime identification
  rewrites for a policy bit would weaken a load-bearing invariant.
- **`RO_COMPAT` or `INCOMPAT` classification** is rejected: read-only
  mounts of a volume with in-place files are entirely safe (the policy
  changes nothing a reader sees), and unlike ADR-064 there is no scenario
  where an unaware implementation corrupts data or loses durability — it
  only ever falls back to stronger full-COW behavior or refuses flagged
  records outright.

## Validation

Acceptance of this ADR gates the implementation, which must land with:

- format: flag constant, spec/afsplus_format.h and feature-registry
  entries, decode acceptance of the known flag, rejection on non-file
  objects;
- congruence: flagged record on a volume without `COMPAT` bit 0 is
  corruption in the mutation path and the checker, with planted-flag
  regressions on both sides, mirroring the ADR-061 tests;
- API: set/get on the VFS layer with persistence across remount (the
  ADR-062 remount-reset test inverts for flagged files), refusal on
  directories and on volumes without the feature;
- behavior: a flagged file takes the in-place path exactly when the
  ADR-062 eligibility rules pass, with the existing power-cut oracles
  re-run over a persistently flagged file rather than the runtime switch.

## Consequences

The ADR-062 architecture becomes deliverable: the owner's per-file choice
survives remounts and travels with the volume, protected from silent loss
by the validated flags namespace, while every implementation that predates
or ignores the feature keeps the stronger default behavior. The runtime
switch shrinks back to what ADR-062 called it — qualification machinery.
