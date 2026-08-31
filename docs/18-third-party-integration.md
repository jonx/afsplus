# 18. Third-Party Integration

> **ADRs:** none · **Spec:** none ·
> **Tests:** none · **Milestones:** none

## 1. Partition identification

AFS+ should request and publish an official GPT partition type GUID before format freeze.

Until assigned, tooling uses a clearly marked experimental GUID.

MBR partition typing is optional legacy support and must not be the primary identification mechanism.

## 2. Filesystem probing

A filesystem probe should need only a fixed small read.

The public identification record exposes:

- magic
- epoch
- UUID
- label
- block size
- basic feature summary
- checksum

## 3. libblkid/file-style support

Provide sample probe code under a permissive license.

The goal is to make basic recognition a small patch for generic tools.

## 4. Partition editors

A partition editor does not need to understand AFS+ merely to create/delete/preserve an AFS+ GPT partition.

Filesystem-aware operations require more:

- minimum shrink size
- resize
- consistency validation

These should be exposed through official tools and library APIs rather than repeatedly reimplemented.

## 5. Resize integration

Provide:

```text
afsplus-min-size
afsplus-resize
```

and library equivalents.

Graphical partition tools can call the official implementation.

## 6. Test vectors

Third-party implementers receive:

- empty volume
- one-file volume
- Unicode volume
- large sparse file
- large directory
- symlink/hardlink volume
- journal replay cases
- unknown feature cases
- corrupt metadata cases

Every vector includes expected inspection output.
