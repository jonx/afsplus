# AFS+ MacAROS Alpha-0 package

> **ADRs:** [ADR-047](../adr/ADR-047-hosted-aros-crash-replay.md),
> [ADR-048](../adr/ADR-048-hosted-aros-system-pivot.md),
> [ADR-049](../adr/ADR-049-hosted-aros-desktop-pivot.md),
> [ADR-051](../adr/ADR-051-explicit-aros-aarch64-platform-profiles.md) · **Spec:** none ·
> **Tests:** none · **Milestones:** none

This directory is produced by [`tools/package-aros-alpha0.sh`](../tools/package-aros-alpha0.sh). Its hashes prove
the exact pre-install set; `check-before.json` proves the image was clean before
target execution.

The files map to a runnable MacAROS tree as follows:

| Package file | Target path |
|---|---|
| `afsplus-handler` | `AROS/L/afsplus-handler` |
| `AFSPlusAlpha0Probe` | `AROS/C/AFSPlusAlpha0Probe` |
| `AFSPlusDosProbe` | `AROS/C/AFSPlusDosProbe` |
| `AFSPlusInfo` | `AROS/C/AFSPlusInfo`, the handler report tool |
| `AFSPlusClone` | `AROS/C/AFSPlusClone`, clone inside an AFS+ volume, byte copy otherwise |
| `FDSKUpdateProbe` | `AROS/C/FDSKUpdateProbe`, the device write-barrier probe of [check-aros-fdsk-ordering.sh](../tools/check-aros-fdsk-ordering.sh) |
| `AFSPlusReplayProbe` | `AROS/C/AFSPlusReplayProbe` |
| `AFSPlusS1Probe` | Stored only in the S1 AFS+ image |
| `AFSPlusS1bProbe` | Stored only in the desktop S1b AFS+ image |
| `AFSPlusS1Pivot` | `AROS/C/AFSPlusS1Pivot` for S1 bootstrap only |
| `AFSPLUS19` | `AROS/Devs/DOSDrivers/AFSPLUS19` |
| `Unit19` | `AROS/DiskImages/Unit19` |
| `abi-report.txt` | AROS ELF identity and `x18`/`TPIDR` machine-code gate |
| `build-profile.txt` | Target, architecture flags and hashes of the Rust target and platform glues |

The DOSDriver describes one 64 MiB device with 16,384 logical 4-KiB blocks and
uses `fdsk.device` unit 19. The 256-KiB handler stack is an Alpha-0 safety value,
not a minimum portability contract. This Hosted fixture explicitly sets
`Mask = 0` because the file-backed device has no DMA-address restriction;
hardware DOSDrivers must advertise and will retain their real DMA mask.

After installing into a dedicated test tree, the target sequence is:

```text
Assign "FDSK:" "SYS:DiskImages"
Mount DEVS:DOSDrivers/AFSPLUS19
AFSPlusAlpha0Probe
```

The fixture is formatted with identification-v3 Unicode 16
`unicode-nfc-casefold`: AROS lookup is case-insensitive while enumeration keeps
the creator's spelling. The probe refuses a dirty/reused fixture, then
exercises create, sparse write, two `ACTION_FLUSH` durability barriers,
truncate, rename, a case-only rename, folded reopen and readback.
On success it leaves `AFSPLUS19:alpha0.from-aros` containing exactly `hello` and
prints:

```text
[AFSPLUS-ALPHA0] PASS create/read/write/truncate/rename/fsync/casefold
```

Stop MacAROS cleanly before copying `Unit19` back to the host. Then require both:

```sh
cargo run --release -p afsplus-check --bin afsplus-check -- /path/to/Unit19 --json
```

Mount the returned image through the host adapter, read
`alpha0.from-aros`, and require the bytes `hello`; then run the host operation
matrix on that same image and check it again. The file lives inside AFS+, not at
a fixed raw offset. The integration smoke script will automate these post-target
gates.

The complete Hosted round trip is automated by:

```sh
tools/check-hosted-aros-alpha0.sh
```

It builds a fresh package, installs only the reserved test artifacts,
runs AROS → host macFUSE → AROS against one image, checks both cross-created
files and the filesystem at every boundary, performs two standard `Mount
AFSPLUS19: SHUTDOWN` plus access-triggered restart cycles through the retained
device node, then performs a final clean shutdown/dismount and removes the
installed artifacts. It refuses to disturb a running Hosted instance or
replace an existing target/result. It also fails up front if the Hosted AROS
`C:Mount` predates the generic `SHUTDOWN` command support required by this
lifecycle.

The handler does not need to be built into AROS. A normal distribution installs
the handler in `L:` and a machine-specific DOSDriver in `DEVS:DOSDrivers`.
Removing that pair after `Mount <device>: SHUTDOWN` followed by `Assign
<device>: DISMOUNT` uninstalls the driver; disk images and physical volumes are
user data and must not be deleted as part of driver removal. The Alpha-0
`Unit19` file is only a disposable qualification fixture.

`build-profile.txt` and `abi-report.txt` are part of `SHA256SUMS`. Profile
format v2 also identifies and hashes the target SDK configuration, host-side
`collect-aros`/`genmodule` tools and ABI auditor. This prevents a package built with
the Darwin-hosted `x18` reservation from being confused with a future
bare-metal profile merely because both binaries use the AArch64 AROS ABI. See
ADR-051.

Deterministic native-handler recovery is automated separately by:

```sh
tools/check-hosted-aros-crash-replay.sh
```

That gate generates modeled power-cut images, boots Hosted MacAROS once per
fixture, runs `AFSPlusReplayProbe old|new`, and requires a clean checker with no
pending intent-log record after every mount. See ADR-047 for the exact coverage
and its limits.

The DOS semantics beyond the Alpha-0 slice are automated by:

```sh
tools/check-hosted-aros-dos.sh
```

`AFSPlusDosProbe` drives the metadata setters, the comment, a soft link,
`ExAll` with a continuation, `OpenFromLock`, `ChangeMode`, record locks,
a `NRF_WAIT_REPLY` notification and `Relabel` through dos.library in one
drawer that it removes. A change made while a notification message is
unreplied must arrive after the reply, never before. The result keeps what
the handler answers to a record lock overlapping the caller's own, as an
observation. A second boot starts `AFSPlusDosProbe RECORD-HOLDER` as its own
task, which keeps a record for three seconds, and runs `RECORD-WAITER`, whose
ten-second waiting request must be granted by that release: later than at
once and well before its timeout. A third boot runs `AFSPlusDosProbe HOLD`, which ends a request
while one of its messages is still unreplied and never replies, and requires
the shutdown and the dismount to succeed and the shell to continue. The first boot also runs
`AFSPlusInfo` against the AFS+ volume, whose JSON report must carry the
`afsplus-handler-info` schema, and against `SYS:`, which must be recognised
as a handler without the extension transport. `AFSPlusClone` must clone a
file inside the volume, refuse to replace the existing target, and fall back
to a byte copy towards `RAM:`; both results must equal the source. The boot ends with
`AFSPlusInfo AFSPLUS19: PACKETS`, the handler's own table of what dos.library
sent it by packet type and of its failures by error code; the result keeps it
as `packets.txt` and requires that the comment arrived as
`ACTION_SET_COMMENT`. A request
that is still registered refuses `ACTION_DIE` by design. The checker must find the
image clean after each boot.

The first post-bootstrap system pivot is automated by:

```sh
tools/check-hosted-aros-s1.sh
```

It builds a content-manifested system subset, installs the bootstrap pivot only
in the old tree, then runs the target-only probe and commands from AFS+.
ADR-048 defines this as S1a. The cumulative desktop and application extension
is S1b.

The complete Hosted desktop extension is automated by:

```sh
tools/check-hosted-aros-s1b.sh
```

It builds the 256 MiB `desktop` profile, pivots the system assigns, verifies the
desktop assigns and durable `ENVARC:` writes, then requires live Wanderer,
IPrefs, Locale and Clock tasks, a populated framebuffer and a clean final
checker. ADR-049 records the accepted evidence. This is a post-bootstrap S1
claim, not an autonomous AFS+ boot-volume claim.
