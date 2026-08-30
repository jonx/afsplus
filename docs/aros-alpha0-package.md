# AFS+ MacAROS Alpha-0 package

This directory is produced by `tools/package-aros-alpha0.sh`. Its hashes prove
the exact pre-install set; `check-before.json` proves the image was clean before
target execution.

The files map to a runnable MacAROS tree as follows:

| Package file | Target path |
|---|---|
| `afsplus-handler` | `AROS/L/afsplus-handler` |
| `AFSPlusAlpha0Probe` | `AROS/C/AFSPlusAlpha0Probe` |
| `AFSPlusReplayProbe` | `AROS/C/AFSPlusReplayProbe` |
| `AFSPlusS1Probe` | Stored only in the S1 AFS+ image |
| `AFSPlusS1bProbe` | Stored only in the desktop S1b AFS+ image |
| `AFSPlusS1Pivot` | `AROS/C/AFSPlusS1Pivot` for S1 bootstrap only |
| `AFSPLUS19` | `AROS/Devs/DOSDrivers/AFSPLUS19` |
| `Unit19` | `AROS/DiskImages/Unit19` |

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

The probe refuses a dirty/reused fixture, then exercises create, sparse write,
two `ACTION_FLUSH` durability barriers, truncate, rename, reopen and readback.
On success it leaves `AFSPLUS19:alpha0.from-aros` containing exactly `hello` and
prints:

```text
[AFSPLUS-ALPHA0] PASS create/read/write/truncate/rename/fsync
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
files and the filesystem at every boundary, and removes the installed artifacts
afterward. It refuses to disturb a running Hosted instance or replace an
existing target/result.

Deterministic native-handler recovery is automated separately by:

```sh
tools/check-hosted-aros-crash-replay.sh
```

That gate generates modeled power-cut images, boots Hosted MacAROS once per
fixture, runs `AFSPlusReplayProbe old|new`, and requires a clean checker with no
pending intent-log record after every mount. See ADR-047 for the exact coverage
and its limits.

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
