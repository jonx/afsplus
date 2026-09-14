# Tools Index

Two kinds of file live here and must not be confused:

- [tools-spec.md](tools-spec.md) specifies the **official CLI tools** that
  ship with AFS+ (`mkafsplus`, `afsplus-info`, `afsplus-check`,
  `afsplus-resize`, `afsplus-dump`, `afsplus-catalog`, `afsplus-fuse`). None of
  the scripts below is one of them.
- `check-*.sh` and the helper scripts are **qualification gates and build
  helpers** for the development machines. They produce evidence sets under
  `build/` and are referenced from the test plans and ADRs that own them.

Every file in this directory has one row here; [`tools/check-docs.py`](check-docs.py)
enforces that.

| File | What it does | Owned by |
|---|---|---|
| [tools-spec.md](tools-spec.md) | Specification of the official CLI tools | [docs/README.md](../docs/README.md) |
| [check-docs.py](check-docs.py) | Documentation contract checker: links, anchors, TOCs, ADR index, navigation blocks, index rows, status rules | [docs/DOCUMENTATION.md](../docs/DOCUMENTATION.md) |
| [test-check-docs.py](test-check-docs.py) | Temporary-fixture tests for deterministic discovery and excluded-tree pruning | [docs/DOCUMENTATION.md](../docs/DOCUMENTATION.md) |
| [check-reservation-portability.sh](check-reservation-portability.sh) | Strict/sanitized portable C reads of original, initialized and fallback reservation images with explicit scratch bounds and optional m68000 compile | [testing/data-policy-qualification.md](../testing/data-policy-qualification.md) |
| [check-portable-c-reader.sh](check-portable-c-reader.sh) | Strict-C99 Rust↔C checkpoint/object/directory/file and durable intent-view cross-read, one-write/one-flush C rename with Rust replay, fault diagnostics, sanitizer, CMake and optional AROS-m68k compile gate | [testing/conformance.md](../testing/conformance.md) |
| [check-portable-c-fuzz.sh](check-portable-c-fuzz.sh) | Records compact Rust-to-C reader and intent-scan paths, runs deterministic ASan/UBSan mutations, exports exact replay artifacts and opportunistically runs libFuzzer | [testing/fuzzing.md](../testing/fuzzing.md) |
| [check-rust-codec-fuzz.sh](check-rust-codec-fuzz.sh) | Runs deterministic raw/resealed/truncated Rust codec mutations, canonical round trips and exact `.afrf` artifact replay | [testing/fuzzing.md](../testing/fuzzing.md) |
| [check-mountable-alpha0.sh](check-mountable-alpha0.sh) | Composite Mountable Alpha-0 completion gate: portable API tests, real macFUSE round trip, Hosted and native AROS matrices, twelve replay cases, one checksummed result set | [ADR-060](../adr/ADR-060-mountable-alpha0-completion-gate.md), M08 |
| [check-hosted-aros-alpha0.sh](check-hosted-aros-alpha0.sh) | S0 bidirectional same-image gate: Hosted MacAROS → host mount → Hosted MacAROS | [testing/aros-system-volume-qualification.md](../testing/aros-system-volume-qualification.md), [ADR-046](../adr/ADR-046-hosted-aros-same-image.md) |
| [check-hosted-aros-crash-replay.sh](check-hosted-aros-crash-replay.sh) | Replays the deterministic intent-log power-cut images through the Hosted MacAROS handler | [testing/aros-system-volume-qualification.md](../testing/aros-system-volume-qualification.md), [ADR-047](../adr/ADR-047-hosted-aros-crash-replay.md) |
| [check-hosted-aros-s1.sh](check-hosted-aros-s1.sh) | S1a: post-bootstrap `SYS:` pivot onto a manifested AFS+ system subset | [testing/aros-system-volume-qualification.md](../testing/aros-system-volume-qualification.md), [ADR-048](../adr/ADR-048-hosted-aros-system-pivot.md) |
| [check-hosted-aros-s1b.sh](check-hosted-aros-s1b.sh) | S1b: desktop, preferences and application session after the `SYS:` pivot | [testing/aros-system-volume-qualification.md](../testing/aros-system-volume-qualification.md), [ADR-049](../adr/ADR-049-hosted-aros-desktop-pivot.md) |
| [check-macaros-native-block-qemu.sh](check-macaros-native-block-qemu.sh) | Qualifies the external writable retained-image transport under native MacAROS QEMU; `alpha0` and `replay` modes mount the off-tree handler and extract the mutated payload from file-backed guest RAM | [docs/aros-native-bridge.md](../docs/aros-native-bridge.md), [ADR-053](../adr/ADR-053-native-macaros-retained-image-transport.md) |
| [check-macaros-native-alpha0-qemu.sh](check-macaros-native-alpha0-qemu.sh) | Native MacAROS Alpha-0 operation matrix under QEMU (`alpha0` mode of the block gate) | [docs/aros-native-bridge.md](../docs/aros-native-bridge.md), [ADR-053](../adr/ADR-053-native-macaros-retained-image-transport.md) |
| [check-macaros-native-replay-qemu.sh](check-macaros-native-replay-qemu.sh) | Replays every deterministic intent-log cut under native MacAROS QEMU and strictly checks the extracted payload | [docs/aros-native-bridge.md](../docs/aros-native-bridge.md), [ADR-054](../adr/ADR-054-native-macaros-crash-replay-extraction.md) |
| [check-aros-m68k-boot-fsuae.sh](check-aros-m68k-boot-fsuae.sh) | Proves the native AROS/m68k boot path under FS-UAE with hash-pinned ROM, floppy and system media, before the handler is added | [ADR-055](../adr/ADR-055-aros-m68k-emulator-gate.md) |
| [check-aros-m68k-alpha0-fsuae.sh](check-aros-m68k-alpha0-fsuae.sh) | Builds the m68k handler, runs the Alpha-0 matrix under FS-UAE on the M68020+ and A500-configured M68000 profiles, then replays every intent-log cut | [docs/aros-native-bridge.md](../docs/aros-native-bridge.md), [ADR-056](../adr/ADR-056-native-aros-m68k-alpha0-and-replay.md), [ADR-057](../adr/ADR-057-plain-m68000-emulator-gate.md) |
| [check-aros-ffi.sh](check-aros-ffi.sh) | Cross-builds and qualifies the native AROS C bridge: translator, trackdisk adapter, static library and handler module | [docs/aros-native-bridge.md](../docs/aros-native-bridge.md), [ADR-042](../adr/ADR-042-aros-c-boundary.md) |
| [check-aros-aarch64-abi.py](check-aros-aarch64-abi.py) | Rejects AROS AArch64 handler artifacts that violate the external ABI or platform profile | [ADR-051](../adr/ADR-051-explicit-aros-aarch64-platform-profiles.md) |
| [check-aros-serial-log.sh](check-aros-serial-log.sh) | Fails when an AROS diagnostic stream contains a modal software-failure requester or another fatal marker | [ADR-059](../adr/ADR-059-guest-failure-diagnostics-are-gate-verdicts.md) |
| [package-aros-alpha0.sh](package-aros-alpha0.sh) | Builds the self-contained, host-checked MacAROS Alpha-0 qualification package; never installs into a MacAROS tree | [docs/aros-alpha0-package.md](../docs/aros-alpha0-package.md) |
| [build-aros-s1-image.sh](build-aros-s1-image.sh) | Builds the manifested AFS+ system image (`core` or `desktop` profile) for the post-bootstrap S1 pivot | [docs/aros-s1-image.md](../docs/aros-s1-image.md) |
| [build-macaros-afsram-device.sh](build-macaros-afsram-device.sh) | Builds the external writable retained-image device for native MacAROS QEMU | [ADR-053](../adr/ADR-053-native-macaros-retained-image-transport.md) |
| [make-macaros-afsram-image.py](make-macaros-afsram-image.py) | Builds the retained MacAROS FAT, AROS-handler and AFS+ RAM image consumed by the native gates | [ADR-053](../adr/ADR-053-native-macaros-retained-image-transport.md) |
| [make-macaros-afsplus-system.py](make-macaros-afsplus-system.py) | Builds the bounded 16-MiB FAT12 bootstrap for native AFS+ QEMU tests | [docs/aros-native-bridge.md](../docs/aros-native-bridge.md) |
| [inject-fat12-file.py](inject-fat12-file.py) | Injects one deterministic 8.3 file into a FAT12 root or subdirectory | [docs/aros-native-bridge.md](../docs/aros-native-bridge.md) |
| [extract-macaros-afsram.py](extract-macaros-afsram.py) | Extracts the unique AFS+ payload from file-backed MacAROS guest RAM for host-side checking | [ADR-054](../adr/ADR-054-native-macaros-crash-replay-extraction.md) |
| [qemu-file-backed-memory.sh](qemu-file-backed-memory.sh) | QEMU wrapper that adds a shared file-backed RAM object to an otherwise standard command line | [ADR-054](../adr/ADR-054-native-macaros-crash-replay-extraction.md) |
| [macos-fskit-modules.sh](macos-fskit-modules.sh) | Inspects and, reversibly, enables the macFUSE FSKit modules when the System Settings switches are inert | [docs/macos-fskit-activation.md](../docs/macos-fskit-activation.md) |
| [third-party-probe-example.c](third-party-probe-example.c) | Pseudocode example of how a generic disk utility identifies an AFS+ volume | [docs/18-third-party-integration.md](../docs/18-third-party-integration.md) |

| [check-backup-tar.sh](check-backup-tar.sh) | Cross-read ordinary tar framing with Python and bsdtar, reproduce the fixture and reject changed payloads in the oracle | [backup archive qualification](../testing/backup-archive-qualification.md) |

| [check-backup-envelope.sh](check-backup-envelope.sh) | Verify envelope hashes/counts independently with OpenSSL and recover body files through Python/bsdtar | [archive envelope](../spec/backup-envelope.md) |
