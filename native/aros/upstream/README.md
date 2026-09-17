# Proposed fixes to generic AROS interfaces

Each entry is a defect in a generic AROS interface found while qualifying the
external AFS+ handler ([ADR-050](../../../adr/ADR-050-external-aros-handler-lifecycle.md)).
A fix is a focused patch against the AROS tree plus a standalone probe that
fails before it and passes after it. AFS+ does not depend on acceptance; a
package that relies on a local copy of a fix states so.

| Patch | Defect | Probe | Built and run |
|---|---|---|---|
| [fdsk-cmd-update.patch](fdsk-cmd-update.patch) | `fdsk.device` answers `CMD_UPDATE` and `ETD_UPDATE` inside `BeginIO`: the reply overtakes writes queued on the unit port and the barrier never reaches the backing file | [fdsk_update_probe.c](../tests/fdsk_update_probe.c) queues a write and a barrier under `Forbid()` and fails when the barrier completes first | Built and run on Hosted darwin-aarch64: without the patch `FAIL CMD_UPDATE replied before the queued CMD_WRITE`, with it `PASS barrier queued behind the write`. Not applied in the gate tree; see the note below |
| [dos-runhandler-serialise.patch](dos-runhandler-serialise.patch) | `RunHandler()` does not serialise its callers. Mount defers a handler's start to the first access, so two tasks that make their first access to a device together each create a handler process: two instances of a filesystem on one medium. It also clears `dn_Task` whenever a startup fails, which unhooks a live handler when the failing one was a second instance. The patch serialises per `DeviceNode`; the handler being started never waits for its own start, a waiter whose starter is no longer a task takes the start over, and waiters receive the starter's result, not `dn_Task` | `AFSPlusDosProbe RECORD-HOLDER` started with `C:Run` next to `RECORD-WAITER`, as the records boot of [check-hosted-aros-dos.sh](../../../tools/check-hosted-aros-dos.sh) does; `aros-ctl tasks` counts the handler tasks | Built (`make kernel-dos`) and run on Hosted darwin-aarch64: before, two `AFSPLUS19` tasks in every run; after, one in three runs of three, none after `Mount SHUTDOWN`, no forwarder started by the AFS+ defence, and the S0 gate with its three handler restarts passes. Not applied in the gate tree, so that the gates keep exercising that defence |
| [m68k-gencall-quad-return.patch](m68k-gencall-quad-return.patch) | `arch/m68k-all/include/gencall.c` emits the `__AROS_LC<n>LONG` and `__AROS_LC<n>double` return aliases and none for `QUAD`, so a library function that both returns a 64-bit integer and takes one pastes `__AROS_LC4QUAD`, which nothing defines. The compiler reports `expected expression before 'QUAD'` at the return type, not at the real subject. No module in the stock tree returns a 64-bit integer by value, so nothing exercises it | Compiling the generated `dos64_regcall_stubs.c` of the DOS64 module for `amiga-m68k`, with the build's own command line | Applied in the m68k SDK tree of this machine: without it the compile fails on `Seek64`, `SetFileSize64`, `Read64` and `Write64`, with it the object builds and carries all ten DOS64 entries. The m68k crosstools build cannot reach `toolchain-linklibs` without it |
| [dos-createnewproc-noexpunge.patch](dos-createnewproc-noexpunge.patch) | On 64-bit, `CreateNewProc()` first tries `AllocMem(MEMF_31BIT)`, which always fails on a host without 31-bit memory; a failing `AllocMem()` runs the low-memory handlers, and lddemon's handler expunges every library and device with an open count of zero, including a device that calls `CreateNewProc()` from its own first open. `fdsk.device` was unloaded while it ran and its code page was reused | Any first open of `fdsk.device`; the S0 gate | Applied in the gate tree of this machine: without it no Hosted gate reaches the handler's first block read (SIGILL inside `fdsk.device`, found with a logging breakpoint on `munmap`). aros-apple-core already carries the same change |

The `CMD_UPDATE` patch is not applied in this machine's gate tree, so the
gates keep running against the device as it is. Applying it is a judgement
for whoever owns the qualification: AFS+ has no defence of its own against a
device that answers a barrier early, unlike the `RunHandler` race below, so
a Hosted run that means to prove ordering needs it.

Known and not patched here:

- The hosted `emul-handler` implements no `ACTION_FLUSH`, so a barrier that
  reaches it cannot become a host `fsync`. With the patch above the device
  treats `ERROR_ACTION_NOT_KNOWN` as the previous behaviour, which is honest
  but means that on Hosted a completed barrier proves ordering inside AROS
  and not that the bytes left the host's page cache.

  Closing it is a decision about a generic DOS interface, not about AFS+, so
  it is written here for whoever takes it upstream rather than patched.
  `ACTION_FLUSH` carries no arguments, and `emul-handler` keeps no list of
  its open files (`struct emulbase`, `arch/all-hosted/filesys/emul_handler`),
  so there are two ways and each costs something.

  *The handler keeps its open files.* `ACTION_FLUSH` then means what it says,
  flush everything, and no interface changes. It costs a list in
  `struct emulbase` maintained by every path that makes or frees a
  `struct filehandle` — three allocation and four release sites in
  `emul_handler.c` today — where one unpaired path corrupts the list of the
  filesystem the whole hosted system runs on. One barrier also `fsync`s every
  open file of the volume, not the one the caller meant.

  *The action takes a file.* `dp_Arg1` would name a file handle, zero keeping
  the old meaning, and a handler would `fsync` exactly that file. It is
  cheap and precise, but it gives an argument to a packet that has none in
  every other implementation, and a handler that ignored the argument and
  answered success would claim a durability it had not performed, which is
  the failure mode the whole chain exists to avoid.

  Either way the host side needs `fsync` in the libc symbol table of
  `arch/all-unix/filesys/emul_handler` (`emul_host_unix.c` and the
  `LibCInterface` of `emul_unix.h`, whose order must stay in step) and a
  `DoFlush()` beside `DoWrite()` in each host backend.
- Upstream `fdsk.device` has no `TD_READ64`, `TD_WRITE64` or `NSCMD_TD_*64`.
  MacAROS carries that fix; images beyond 4 GiB depend on it.
