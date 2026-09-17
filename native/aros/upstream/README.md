# Proposed fixes to generic AROS interfaces

Each entry is a defect in a generic AROS interface found while qualifying the
external AFS+ handler ([ADR-050](../../../adr/ADR-050-external-aros-handler-lifecycle.md)).
A fix is a focused patch against the AROS tree plus a standalone probe that
fails before it and passes after it. AFS+ does not depend on acceptance; a
package that relies on a local copy of a fix states so.

| Patch | Defect | Probe | Built and run |
|---|---|---|---|
| [fdsk-cmd-update.patch](fdsk-cmd-update.patch) | `fdsk.device` answers `CMD_UPDATE` and `ETD_UPDATE` inside `BeginIO`: the reply overtakes writes queued on the unit port and the barrier never reaches the backing file | [fdsk_update_probe.c](../tests/fdsk_update_probe.c) queues a write and a barrier under `Forbid()` and fails when the barrier completes first | Applies to the AROS source tree; needs a Hosted AROS build to compile and run |

Known and not patched here:

- The hosted `emul-handler` implements no `ACTION_FLUSH`, so a barrier that
  reaches it cannot become a host `fsync`. With the patch above the device
  treats `ERROR_ACTION_NOT_KNOWN` as the previous behaviour; host durability
  on Hosted needs that handler to map the action to `fsync` on its open
  files.
- Upstream `fdsk.device` has no `TD_READ64`, `TD_WRITE64` or `NSCMD_TD_*64`.
  MacAROS carries that fix; images beyond 4 GiB depend on it.
