# ADR-078: Distinguish full backup preservation from content recovery

Status: Accepted by the owner; profile and destination qualification required
Amends: ADR-076

## Context

AFS+ preallocation reserves unwritten blocks without extending logical file
size. A content-only stream omits these reservations, including those beyond
EOF. Applications can use reservations to reduce later allocation failures.
Preserving them consumes destination capacity and requires equivalent provider
support. The owner selected two explicit modes after discussing that trade-off.

## Decision

Define full preservation and content recovery as separate archive-consumer
outcomes. Full preservation includes logical contents, holes and unwritten
preallocation, including reservations beyond EOF, alongside the metadata
required by ADR-076. Require equivalent destination reservation semantics or
refuse full preservation explicitly. Never convert an unsupported reservation
into an unreported hole or written zero data.

Content recovery restores readable contents and supported namespace semantics
while explicitly reporting discarded reservations and other preservation losses.
It must not report full preservation. Generic tar recovery is qualified for its
supported subset; successful ordinary extraction cannot imply reservation or
security-metadata preservation.

Record allocation ranges semantically in bytes, independently of physical
addresses, tree records and sharing layout. File size is a separate value.
The exact profile must define alignment and rounding behavior, written tails,
reservation identity across hard links, and unsupported-state reporting.
Enumeration is bounded and bound to the retained source snapshot. Unknown
required metadata follows ADR-076 refusal or lossless transport rules.

Reservation preservation promises the destination provider's qualified capacity
reservation semantics. It does not guarantee that every subsequent application
write succeeds despite metadata exhaustion, device errors or revoked authority.
Do not advertise an unqualified host reservation primitive as equivalent.

## Compatibility and qualification

This selects archive behavior and changes no AFS+ disk records or feature bits.
The exact PAX encoding, restore ordering and completion evidence follow the
profile/API process. Native provider support requires independent qualification.

Compare files with holes, written zero blocks, unwritten extents inside EOF,
reservations beyond EOF, rounded final blocks and shared hard-link identities.
Full preservation must restore equivalent reservations or refuse unsupported
providers. Content recovery must preserve readable bytes and report losses.
Test remount, snapshot stability, bounded traversal over huge gaps, explicit
page limits, revocation and partial output. Keep physical allocation addresses
out of the application interface and archive.
