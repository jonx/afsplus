# ADR-050: Keep the AROS handler external and qualify its unload/reload lifecycle

Status: Accepted for Hosted and native-QEMU lifecycle

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
Adding AFS+ itself to the AROS source tree is not required. An AROS or
distribution build recipe may be contributed later, but acceptance of that
recipe is not a release condition for AFS+.

This is not a rule against fixing AROS. If conformance testing exposes a defect
in a generic Exec, DOS, device or filesystem interface, the preferred result is
a minimal regression-tested fix proposed upstream. Until it is accepted, a
MacAROS build may carry that fix locally and the package must state the exact
dependency. A private compatibility shim is appropriate only when it implements
a genuinely platform-specific boundary; it must not hide a general AROS bug.

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

ADR-052 requalification repeated both standard dismount/remount cycles after a
case-only target rename. The final case-insensitive image was checker-clean at
generation 22 with three objects and zero pending intent records.

ADR-053 then exercised the same shutdown ordering through the native
Apple-AArch64 handler under QEMU. The handler completed cleanup before its
deferred death reply, its process disappeared, and both handler and block
device unloaded before QEMU exited cleanly.

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
