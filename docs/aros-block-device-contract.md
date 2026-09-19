# What the AFS+ handler requires of an AROS block device

> **ADRs:** [ADR-044](../adr/ADR-044-aros-trackdisk-viewport.md),
> [ADR-122](../adr/ADR-122-aros-partition-identity.md) ·
> **Milestones:** —

For the authors of AROS disk drivers, in particular the USB mass-storage and
NVMe drivers of Macaros Native. The AFS+ handler is only as crash-safe as the
device below it: this page states what it relies on, and
[`AFSPlusDriverProbe`](../native/aros/tools/afsplus_driver_probe.c) checks
each clause on a running system. Every clause marked required must hold
before an AFS+ volume, or a boot partition, is put on the device.

## How the handler uses the device

- One `IOExtTD` request, issued with `DoIO`: one request at a time, each
  finished before the next is sent.
- Every transfer is one 4096-byte logical block at a 4096-byte multiple of
  the partition. The partition start is a multiple of the device's sector
  size, and 4096 is too.
- Offsets are 64-bit: `io_Offset` holds the low word and `io_Actual` the
  high word, as TD64 and the NSD 64-bit commands define.

## Commands

| Command | Required | What the handler relies on |
|---|---|---|
| `NSCMD_DEVICEQUERY` | no | Lists `NSCMD_TD_READ64` and `NSCMD_TD_WRITE64` when the device has them; the handler prefers them |
| `NSCMD_TD_READ64`, `NSCMD_TD_WRITE64` or `TD_READ64`, `TD_WRITE64` | for a partition that ends above 4 GiB | Without them the handler refuses such a partition rather than misaddress it |
| `CMD_READ`, `CMD_WRITE` | when the 64-bit commands are missing | Offsets below 4 GiB only |
| `CMD_UPDATE` | yes | See durability below |
| `TD_GETGEOMETRY` | yes, for the probe and the boot scan | The sector size divides 4096; the total is exact |
| `TD_CHANGESTATE` | no | `io_Actual` 0 when a medium is present; an unknown command counts as present |
| `TD_PROTSTATUS` | no | `io_Actual` non-zero when write-protected; an unknown command counts as writable, and a protected medium mounts read-only |

## Clauses

1. **Exact transfers (required).** A read or write of 4096 bytes transfers
   exactly that, sets `io_Actual` to 4096 and `io_Error` to 0, or fails with
   a non-zero `io_Error`. A short transfer is a failure.
2. **Reads return what was written (required).** After a successful write and
   `CMD_UPDATE`, a read of the same block returns the written bytes.
3. **The last write wins (required).** Two writes to one block, in order,
   leave the second.
4. **Durability (required).** When `CMD_UPDATE` replies without error, every
   write that completed before it is on stable storage: a power cut after the
   reply loses none of them. For a file-backed device this means the host
   file is synced; for NVMe, a flush command and its completion; for USB mass
   storage, SYNCHRONIZE CACHE. The handler's checkpoints and its intent log
   depend on this clause and on nothing else for crash safety.
5. **The barrier (required).** A `CMD_UPDATE` queued behind a write that is
   still pending does not reply before that write. The handler itself waits
   for each write, but a device that answers `CMD_UPDATE` early is also one
   that can answer it without reaching the medium.
6. **Bounds (required).** A transfer that starts at or beyond the end of the
   device fails; it never wraps round or lands elsewhere.
7. **Memory.** `de_Mask` and `de_BufMemType` say which buffers the device can
   take. The handler bounces a buffer the mask excludes. On 64-bit AROS a
   mask with no bit above 31 constrains only the low word, as PFS3 and SFS
   read it, so a device that can reach all memory should give a full mask or
   none.

## Checking a device

```
AFSPlusDriverProbe <device> <unit> <first-block> <blocks> WRITE
```

Blocks are 4096 bytes. The probe reads the range and keeps it, writes
distinct patterns, checks clauses 1 to 3, 5 and 6, and writes the range back
as it was; `WRITE` is the consent. Clause 4 cannot be seen without cutting
the power: it is checked by the crash matrices on a target. Each clause
prints `PASS` or `FAIL`, and the last line is the verdict.

On Hosted MacAROS,
[`check-hosted-aros-driver.sh`](../tools/check-hosted-aros-driver.sh) runs
the probe on `fdsk.device` and `hostdisk.device`. `fdsk.device` fails clause
5: it answers `CMD_UPDATE` inside `BeginIO`, a known defect with an upstream
patch
([stage-c-gap C12](../implementation/stage-c-gap.md#c12-file-backed-virtual-block-device)),
and the check records it instead of failing on it.

`hostdisk.device` passes clause 5 with the local patch
[`native/aros/aros-patches/hostdisk-unit-pattern.patch`](../native/aros/aros-patches/hostdisk-unit-pattern.patch):
`CMD_UPDATE` is queued on the unit port like `CMD_WRITE`, so it is answered
behind the writes ahead of it, and the unit thread then syncs the host file,
`fcntl(fd, F_FULLFSYNC)` on Darwin with `fsync()` as the fallback and on
every other host. A failing sync becomes a non-zero `io_Error`. A barrier
failure of `hostdisk.device` is therefore a failure of the check, and a
native driver should pass every clause.
