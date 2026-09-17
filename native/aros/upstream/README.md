# Proposed fixes to generic AROS interfaces

Each entry is a defect in a generic AROS interface found while qualifying the
external AFS+ handler ([ADR-050](../../../adr/ADR-050-external-aros-handler-lifecycle.md)).
A fix is a focused patch against the AROS tree plus a standalone probe that
fails before it and passes after it. AFS+ does not depend on acceptance; a
package that relies on a local copy of a fix states so.

| Patch | Defect | Probe | Built and run |
|---|---|---|---|
| [fdsk-cmd-update.patch](fdsk-cmd-update.patch) | `fdsk.device` answers `CMD_UPDATE` and `ETD_UPDATE` inside `BeginIO`: the reply overtakes writes queued on the unit port and the barrier never reaches the backing file | [fdsk_update_probe.c](../tests/fdsk_update_probe.c) queues a write and a barrier under `Forbid()` and fails when the barrier completes first | Applies to the AROS source tree; needs a Hosted AROS build to compile and run |
| [dos-runhandler-serialise.patch](dos-runhandler-serialise.patch) | `RunHandler()` does not serialise its callers. Mount defers a handler's start to the first access, so two tasks that make their first access to a device together each create a handler process: two instances of a filesystem on one medium. It also clears `dn_Task` whenever a startup fails, which unhooks a live handler when the failing one was a second instance. The patch serialises per `DeviceNode`; the handler being started never waits for its own start, a waiter whose starter is no longer a task takes the start over, and waiters receive the starter's result, not `dn_Task` | `AFSPlusDosProbe RECORD-HOLDER` started with `C:Run` next to `RECORD-WAITER`, as the records boot of [check-hosted-aros-dos.sh](../../../tools/check-hosted-aros-dos.sh) does; `aros-ctl tasks` counts the handler tasks | Built (`make kernel-dos`) and run on Hosted darwin-aarch64: before, two `AFSPLUS19` tasks in every run; after, one in three runs of three, none after `Mount SHUTDOWN`, no forwarder started by the AFS+ defence, and the S0 gate with its three handler restarts passes. Not applied in the gate tree, so that the gates keep exercising that defence |
| [dos-createnewproc-noexpunge.patch](dos-createnewproc-noexpunge.patch) | On 64-bit, `CreateNewProc()` first tries `AllocMem(MEMF_31BIT)`, which always fails on a host without 31-bit memory; a failing `AllocMem()` runs the low-memory handlers, and lddemon's handler expunges every library and device with an open count of zero, including a device that calls `CreateNewProc()` from its own first open. `fdsk.device` was unloaded while it ran and its code page was reused | Any first open of `fdsk.device`; the S0 gate | Applied in the gate tree of this machine: without it no Hosted gate reaches the handler's first block read (SIGILL inside `fdsk.device`, found with a logging breakpoint on `munmap`). aros-apple-core already carries the same change |

Known and not patched here:

- The hosted `emul-handler` implements no `ACTION_FLUSH`, so a barrier that
  reaches it cannot become a host `fsync`. With the patch above the device
  treats `ERROR_ACTION_NOT_KNOWN` as the previous behaviour; host durability
  on Hosted needs that handler to map the action to `fsync` on its open
  files.
- Upstream `fdsk.device` has no `TD_READ64`, `TD_WRITE64` or `NSCMD_TD_*64`.
  MacAROS carries that fix; images beyond 4 GiB depend on it.
