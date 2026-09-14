# ADR-089: Verify restored allocation through scoped bounded readback

Status: Accepted under the owner's delegated recommended-option authority; complete allocation-preservation consumer required
Amends: ADR-077, ADR-078

## Context

A successful write or reservation call alone cannot prove that a destination
with different allocation granularity preserved the source holes, written zero
ranges and unwritten reservations. Total allocated bytes do not identify where
that coverage resides. Full-preservation acceptance needs semantic readback.

## Decision

Add bounded allocation enumeration to the separately authorized restore
interface. Return ordered byte ranges with written/unwritten state and an
entry-ordinal cursor, excluding physical addresses and sharing hints. Queries
use the original object's revocable grant on every operation. Validate request
limits and provider response progress, range ordering and overflow. An absent
provider implementation returns unsupported, never an empty successful inventory.

The AFS+ provider exposes committed file allocation through the same bounded
extent-page mechanism used by captured snapshot reads. The live query refuses
an open intent-log window rather than silently returning stale committed data
or flushing as a side effect of a read. A caller must complete its mutation
window before requesting committed readback. It performs no source writes.

The complete allocation-preservation consumer must compare exact byte coverage
and written/unwritten state after restoration, allowing equivalent adjacent
segmentation. It must refuse an unsupported or mismatched destination. Logical
file size is checked separately. Content recovery need not claim allocation
preservation and must report discarded reservations under ADR-078.

## Qualification

Compare empty, written, sparse, unwritten, rounded-tail and beyond-EOF layouts
through one-entry and larger pages, then remount. Snapshot results must keep their
captured layouts while committed live readback follows mutations. Exercise
invalid cursors/limits, unsupported providers, wrong/revoked grants and malformed
responses. Queries must issue no writes or barriers. Pending-window refusal must
not flush or expose a fabricated empty map. No on-disk format or C ABI changes
follow, and native provider qualification is separate.
