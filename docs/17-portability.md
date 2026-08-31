# 17. Portability

> **ADRs:** none · **Spec:** none ·
> **Tests:** [aros-system-volume-qualification](../testing/aros-system-volume-qualification.md),
> [conformance](../testing/conformance.md), [fuzzing](../testing/fuzzing.md) · **Milestones:** M01, M08

## 1. Portable core requirement

`libafsplus` must build outside AROS.

The core may depend only on explicit portability shims for:

- block I/O
- memory allocation
- locking
- time
- logging
- random UUID generation where formatting

## 2. Host-file backend

The first backend should treat an ordinary host file as a block device.

This enables development on macOS/Linux:

```text
mkafsplus image.afsp 10G
afsplus-check image.afsp
afsplus-dump image.afsp
```

before the AROS handler exists.

## 3. FUSE

FUSE support is a first-class development objective.

Benefits:

- inspect AROS system disks from the M5 development Mac
- copy binaries without booting AROS
- run conformance tests using normal host tools
- make the format accessible to Linux/macOS users
- force clean separation between filesystem core and AROS handler

## 4. Windows

Native Windows support is not required for 1.0.

The specification, reader library, and conformance suite should make a future Windows implementation straightforward.

## 5. Other Amiga-family systems

Porting should require:

- block-provider glue
- filesystem handler glue
- encoding/path integration

It should not require reverse engineering the format from AROS source.
