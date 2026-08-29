# 13. Filesystem API v2

## 1. Purpose

Filesystem API v2 provides modern operations without breaking the classic AROS DOS ABI.

Classic applications continue to call existing DOS APIs.

Internally:

```text
classic DOS call -> compatibility translation -> 64-bit filesystem operation
modern API call  -> direct modern operation
```

## 2. Design principles

- 64-bit clean
- capability-driven
- filesystem-neutral
- iterator/stream oriented for large result sets
- no private AFS+ assumptions
- stable object identity
- explicit durability
- explicit encoding
- structured errors

## 3. Core operations

Required categories:

### handles and I/O

- open
- close
- read
- write
- seek64
- truncate64
- flush file

### namespace

- create
- mkdir
- unlink
- rmdir
- rename
- atomic replace
- link
- symlink
- readlink

### metadata

- stat64
- statfs64
- get/set protection
- get/set comment
- get/set xattr
- object ID

### enumeration

- directory iterator
- full-volume object iterator
- change-stream iterator

### synchronization

- fsync file
- fsync directory
- sync filesystem

### observation

- watch
- query capabilities
- query limits

## 4. Compatibility adapters

A legacy handler can expose v2 through an adapter.

Unsupported capabilities return a precise `NOT_SUPPORTED` result.

Applications must not infer capability from filesystem name.

## 5. Large files

Classic APIs that accept signed 32-bit offsets retain their historical limits.

The v2 API uses 64-bit offsets and lengths end to end.

## 6. Rust

The AROS Rust `std` port should bind to v2 for modern file operations.

Rust `Metadata::len()` maps naturally to 64-bit size.

Rust applications must not need AFS+-specific code.

## 7. Zed

Zed-facing requirements include:

- canonical path resolution
- symlink-safe resolution
- stable object IDs
- reliable rename
- recursive watch behavior or equivalent
- 64-bit metadata
- efficient large-directory enumeration

These are platform API requirements, not AFS+ private APIs.
