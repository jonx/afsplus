# AFS+ MacAROS Alpha-0 package

This directory is produced by `tools/package-aros-alpha0.sh`. Its hashes prove
the exact pre-install set; `check-before.json` proves the image was clean before
target execution.

The files map to a runnable MacAROS tree as follows:

| Package file | Target path |
|---|---|
| `afsplus-handler` | `AROS/L/afsplus-handler` |
| `AFSPlusAlpha0Probe` | `AROS/C/AFSPlusAlpha0Probe` |
| `AFSPLUS19` | `AROS/Devs/DOSDrivers/AFSPLUS19` |
| `Unit19` | `AROS/DiskImages/Unit19` |

The DOSDriver describes one 64 MiB device with 16,384 logical 4-KiB blocks and
uses `fdsk.device` unit 19. The 256-KiB handler stack is an Alpha-0 safety value,
not a minimum portability contract.

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
cargo run --release -p afsplus-check -- /path/to/Unit19 --json
```

Mount the returned image through the host adapter, read
`alpha0.from-aros`, and require the bytes `hello`; then run the host operation
matrix on that same image and check it again. The file lives inside AFS+, not at
a fixed raw offset. The integration smoke script will automate these post-target
gates.
