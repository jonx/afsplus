# ADR-058: Bound M68000 emulator memory and qualify stable handler restarts

Status: Accepted

## Context

ADR-057 proved plain-M68000 functionality and recovery with 8 MiB of Fast RAM,
but did not separate the AROS baseline, the loaded handler segment and the
per-instance filesystem state. The original gate also used `Assign DISMOUNT`
as if it stopped the handler. Current AROS deliberately splits the lifecycle:
`Mount <device>: SHUTDOWN` sends `ACTION_DIE` and retains the `DeviceNode`,
while `Assign <device>: DISMOUNT` removes an already inactive node.

Removing the node without first stopping AFS+ orphaned a live handler. Removing
and recreating the node for every restart also forced DOS to load another copy
of the external segment, obscuring the real steady-state cost.

## Decision

The native handler now completes all owned cleanup before replying to
`ACTION_DIE`, clears its task from the `DeviceNode`, and leaves that node to the
caller. Qualification uses two access-triggered handler instances through the
same retained node, followed by one final `Assign DISMOUNT`. Every replay case
also requires successful shutdown and dismount before host extraction.

The Alpha-0 guest records `Avail FLUSH` before mount, while mounted, after the
first shutdown, after restart, after the second shutdown and after final
dismount. Every Alpha-0 and replay serial log must also pass the software-failure
requester scanner. The accepted A500/M68000 run measured these Fast-RAM values:

| State | Available bytes |
|---|---:|
| Before first mount | 7,948,840 |
| First instance active | 4,922,032 |
| After first shutdown | 5,998,568 |
| Restarted instance active | 4,921,856 |
| After second shutdown | 5,998,504 |
| After final dismount | 5,998,584 |

The first active instance therefore consumes 3,026,808 bytes relative to the
AROS baseline. Approximately 1,950,272 bytes remain with the DOS-loaded
external segment, while an active handler instance adds approximately
1,076,536 bytes. The two post-shutdown samples differ by only 64 bytes, so the
qualified restart cycle has no material per-instance memory growth.

An A500-configured boot-only gate passes with 8 MiB Fast RAM. Both the complete
AFS+ gate and a boot-only control timed out at 4 MiB before the Startup-Sequence
wrote its first verdict, without an AROS software-failure requester. This makes
8 MiB the first qualified FS-UAE Fast-RAM step for this official AROS image; it
does **not** prove that AFS+ itself requires 8 MiB, because the 4 MiB control
never reached AFS+.

## Consequences

- The classic Rust reference handler now has a measured functional memory
  profile and a stable sequential restart lifecycle under emulation.
- Reusing the retained `DeviceNode` is the normal restart path. Repeatedly
  removing and recreating a path-loaded node can retain additional loader
  segments on current AROS and is not used as a steady-state benchmark.
- The external segment retained after final dismount is a generic AROS loader
  lifecycle question, not hidden as filesystem heap. Any change to unload it
  must be fixed and regression-tested at the generic AROS boundary.
- This result sets neither a physical-A500 RAM requirement nor an acceptable
  CPU/performance budget. A smaller `no_std + alloc` or portable-C profile
  remains the fallback for systems below the Rust/AROS reference profile.
