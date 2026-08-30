# ADR-050: Keep the AROS handler external and qualify its unload/reload lifecycle

Status: Accepted for Hosted lifecycle

## Context

AFS+ must not depend on the AROS project accepting a new filesystem into its
source tree. AROS already supports external filesystem handlers through a
binary in `L:`, a `DEVS:DOSDrivers` entry and ordinary Exec/DOS packets.

The first Alpha-0 handler understood `ACTION_DIE`, but the standard command
`Assign AFSPLUS19: DISMOUNT` sends `ACTION_INHIBIT` first. It also expects a
successful handler to remove its device node. Replying to `ACTION_DIE` before
cleanup allowed the caller to free or replace DOS state while the old handler
still referenced it.

## Decision

The AROS integration remains an independently built and distributed module.
No change to the AROS kernel, `dos.library` or source tree is required. An AROS
or distribution build recipe may be contributed later, but acceptance of that
recipe is not a release condition for AFS+.

The packet translator now implements `ACTION_INHIBIT`:

- inhibit refuses while a native lock or file is outstanding;
- the first inhibit flushes the filesystem before acknowledging it;
- repeated inhibit is idempotent; and
- uninhibit restores the pre-shutdown state if a later death request fails.

On a successful `ACTION_DIE`, the native loop withholds the reply, dismantles
the packet context, DOS volume, Rust mount and backing device, removes the
device node from the DosList, closes its libraries and only then replies. It
does not free the device-node storage while the initiating DOS command may
still reference it.

The Hosted same-image gate performs two consecutive
`Assign AFSPLUS19: DISMOUNT`/`Mount` cycles. Every command records an explicit
status and each fresh handler must read bytes committed through the prior
instances.

## Evidence

The lifecycle gate passed on 2026-08-30. Both dismounts and both remounts
reported `pass`; the first fresh instance read `alpha0.from-host` as `host` and
the second read `alpha0.from-aros` as `hello`. The final strict checker reported
generation 21, three objects, no warnings or errors, and zero pending intent-log
records.

The same change remains cross-qualified against genuine AROS headers and the
complete AArch64 handler link, plus the m68k packet, trackdisk, handler and
generated-entry ABI compilation gates.

## Consequences

An AFS+ release can be installed on a compatible AROS distribution without
upstream coordination. MacAROS may bundle or offer the same external package,
while another distribution can keep it optional. Uninstall removes the handler
and DOSDriver after a clean dismount; it never implies deleting filesystem
images or physical-volume data.

Hot-removable media is still a separate contract. `TD_ADDCHANGEINT`, forced
removal with outstanding locks and media replacement remain disabled until
their races have dedicated tests.
