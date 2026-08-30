# ADR-052: Put versioned directory comparison keys on disk

Status: Accepted for the executable prototype

## Context

The initial tree stored original UTF-8 bytes as its key. That made every volume
case-sensitive and canonically sensitive even though AFS+ promised preserved
spelling with configurable lookup. Implementing folding only in the AROS
adapter would permit duplicates through FUSE and would make on-disk ordering
depend on which OS created an entry.

Unicode normalization and case-fold mappings also change between releases.
Calling a host lowercase or locale API cannot define persistent keys.

## Decision

Identification version 3 records a comparison-key algorithm and a Unicode
major/minor/patch version. New prototype volumes use Unicode 16.0.0:

- `unicode-nfc` preserves case and encodes NFC;
- `unicode-nfc-casefold` applies NFD, full default non-Turkic case folding and
  NFC recomposition; and
- `legacy-identity` is decoded for version-1/2 prototype images but is never
  emitted by the new formatter.

The B+ tree continues to compare the resulting bytes unsigned and never needs
locale collation. Leaves retain the exact valid UTF-8 spelling supplied by the
creator. A case-only or canonically equivalent rename updates that spelling
without creating a second key.

General `afsplus-mkfs` defaults to the sensitive policy. AROS qualification
images explicitly select the insensitive policy. The checker schema reports
the algorithm, case behavior and Unicode version, and validates every stored
key against its original name during the exhaustive sweep.

## Prototype evidence

The portable core, VFS, FUSE protocol and AROS adapters reject folded
duplicates, preserve creator spelling and accept canonically equivalent lookup.
An exhaustive crash matrix proves a case-only rename is either wholly absent or
fully replayed with its new spelling. Hosted S0 accepts a folded reopen after a
case-only rename, and Hosted S1b starts the desktop with stock `THEME:Images`
against the stored `THEME:images` path. All returned images pass checker schema
5 with no pending intent record.

In the current AArch64 Alpha-0 link, adding the Unicode tables increased the
handler from 2,918,440 to 3,200,904 bytes (+282,464 bytes, 9.7%). This is a
prototype binary-size measurement, not a classic-machine acceptance result.
The release qualification also built and revalidated a 100,000-entry
case-insensitive directory in 7.74 seconds, then streamed its 2,011-node tree in
47.8 milliseconds while retaining the existing eight staged/two decoded-page
bounds. These are host measurements; emulator and physical-A500 budgets remain
separate gates.

## Consequences

AROS, VFS and FUSE now share one namespace definition, so an entry created by
one adapter cannot be duplicated under folded or canonically equivalent
spelling through another. Existing prototype images keep their exact old
semantics instead of being silently reinterpreted.

Unicode tables add code and data to writers. Classic-machine qualification
must measure that cost and may use a separately linked table provider, but it
must not silently substitute a different folding algorithm. Per-directory
policy overrides and the epoch-1 Unicode/table freeze remain later decisions.
