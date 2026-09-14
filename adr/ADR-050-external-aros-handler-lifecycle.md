# ADR-050: Keep the AROS handler external and qualify its unload/reload lifecycle

Status: Accepted

## Context

AFS+ must not depend on the AROS project accepting a new filesystem into its
source tree. AROS already supports external filesystem handlers through a
binary in `L:`, a `DEVS:DOSDrivers` entry and ordinary Exec/DOS packets.

The first Alpha-0 handler understood `ACTION_DIE`, but older AROS
`Assign AFSPLUS19: DISMOUNT` combined stopping the handler with removing its
device node. Current AROS deliberately separates those operations: `Mount
AFSPLUS19: SHUTDOWN` stops the handler while retaining the node, and `Assign
AFSPLUS19: DISMOUNT` removes the inactive node without waking it. Replying to
`ACTION_DIE` before cleanup would still allow the caller to replace DOS state
while the old handler referenced it.

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
the packet context, DOS volume, Rust mount and backing device, clears its task
from the device node, closes its libraries and only then replies. It leaves the
device node in the DosList: the caller owns the documented shutdown/dismount
split, and an ordinary shutdown must permit a later access to restart the
handler from the retained node.

The Hosted same-image gate performs two consecutive `Mount SHUTDOWN`/restart
cycles through the retained device node plus a final clean shutdown/dismount.
Every command records an explicit status and each fresh handler must read bytes
committed through the prior instances. The m68k emulator gate applies a clean
shutdown/dismount before extracting and checking every image.

## Evidence

The original lifecycle gate passed on 2026-08-30. Both dismounts and remounts
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

The split lifecycle was requalified on Hosted AROS on 2026-08-31 after building
the accepted generic AROS changes `e692899b10` (Assign removes an inactive
node) and `657f4ec4bd` (Mount adds `SHUTDOWN`). Two access-triggered restarts,
both cross-created-file readbacks, three shutdowns and the final dismount all
reported `pass`; the strict checker remained clean after both platform
boundaries and at the end. The gate now rejects a Hosted `C:Mount` binary whose
command template lacks `SHUTDOWN` instead of hanging against stale tools.

The same change remains cross-qualified against genuine AROS headers and the
complete AArch64 handler link, plus the m68k packet, trackdisk, handler and
generated-entry ABI compilation gates.

## Consequences

An AFS+ release can be installed on a compatible AROS distribution without
upstream coordination. MacAROS may bundle or offer the same external package,
while another distribution can keep it optional. Uninstall removes the handler
and DOSDriver after the explicit shutdown/dismount pair; it never implies
deleting filesystem images or physical-volume data. Distributions predating
the split `Mount SHUTDOWN`/`Assign DISMOUNT` contract need the corresponding
generic AROS command update rather than an AFS+-specific lifecycle shim.

Hot-removable media is still a separate contract. `TD_ADDCHANGEINT`, forced
removal with outstanding locks and media replacement remain disabled until
their races have dedicated tests.
