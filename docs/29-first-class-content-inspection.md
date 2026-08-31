# 29. First-Class Content Inspection and Anti-Malware Support

> **ADRs:** none · **Spec:** none ·
> **Tests:** none · **Milestones:** none

<!-- toc -->

- [1. Goal](#1-goal)
- [2. Lessons from existing systems](#2-lessons-from-existing-systems)
- [3. Core principle: scan once per content generation](#3-core-principle-scan-once-per-content-generation)
- [4. Durable whole-filesystem security feed](#4-durable-whole-filesystem-security-feed)
- [5. Coalesce writes, do not notify every write syscall by default](#5-coalesce-writes-do-not-notify-every-write-syscall-by-default)
- [6. Race-free scanning by object and generation](#6-race-free-scanning-by-object-and-generation)
- [7. Fast execution/open authorization](#7-fast-executionopen-authorization)
- [8. Clone/reflink awareness](#8-clonereflink-awareness)
- [9. Optional content fingerprint cache](#9-optional-content-fingerprint-cache)
- [10. Security verdicts should not be ordinary file metadata](#10-security-verdicts-should-not-be-ordinary-file-metadata)
- [11. Whole-volume initial scan without pathname overhead](#11-whole-volume-initial-scan-without-pathname-overhead)
- [12. Priority, filtering, and backpressure](#12-priority-filtering-and-backpressure)
- [13. Changed-range hints](#13-changed-range-hints)
- [14. Security origin metadata](#14-security-origin-metadata)
- [15. Resource target](#15-resource-target)
- [16. Broader value](#16-broader-value)

<!-- /toc -->

## 1. Goal

AFS+ should make whole-filesystem security monitoring cheap enough that an antivirus, malware scanner, backup engine, indexer, DLP engine, or integrity monitor does not need to rediscover the filesystem by repeatedly walking paths and reopening unchanged files.

The goal is not to embed antivirus logic in the filesystem. The goal is to provide a first-class, filesystem-neutral content-inspection contract that lets trusted security software observe exactly what changed, read the exact content generation that triggered an event, cache its verdict safely, and block sensitive accesses only when necessary.

## 2. Lessons from existing systems

Modern operating systems expose powerful interception facilities:

- Windows file-system minifilters can observe or block file I/O and are commonly used by antivirus products.
- Linux fanotify can monitor an entire filesystem and can issue permission events before access or execution.
- macOS Endpoint Security provides notification and authorization events for security software.

These mechanisms are powerful, but placing complex scanning logic on every open/read path can create significant latency and CPU overhead.

AFS+ should optimize the common case where content has already been scanned and has not changed.

## 3. Core principle: scan once per content generation

Every regular file has:

- stable object ID
- monotonically changing content generation
- metadata generation where useful

Conceptually:

```text
filesystem UUID = F
object ID       = 0x1234
content gen     = 87
```

A security engine may cache a verdict against:

```text
(F, object_id, content_generation, scanner_engine_version, signature_set_version)
```

If the file is opened again and all of these remain compatible, no data rescan is required.

Changing permissions or renaming the file does not automatically change the content generation.

Any operation that changes file bytes does.

## 4. Durable whole-filesystem security feed

The existing AFS+ persistent change stream should support a privileged security subscription with records such as:

```text
CONTENT_CREATED
CONTENT_GENERATION_CHANGED
OBJECT_CLONED
OBJECT_DELETED
OBJECT_RENAMED
EXECUTABLE_INTENT
SECURITY_METADATA_CHANGED
ORIGIN_METADATA_CHANGED
```

Security consumers receive:

- filesystem UUID
- object ID
- previous/new content generation
- transaction/checkpoint generation
- object type
- relevant metadata summary
- optional changed byte ranges when cheaply available
- origin/quarantine flags where the OS provides them

The stream is persistent and cursor-based. A scanner can stop for a day, restart, and ask what changed since sequence N without walking the entire namespace.

If the retention window has expired, it receives `RESCAN_REQUIRED` and can use `EnumerateObjects()` as the fallback.

## 5. Coalesce writes, do not notify every write syscall by default

A scanner generally cares that file content became a new committed version, not that an application issued 6,000 individual 4 KiB writes while saving it.

Default security events should therefore be transaction/content-generation oriented.

Example:

```text
open file for write
write 4 KiB
write 4 KiB
write 1 MiB
write 32 KiB
close/fsync/commit

=> one CONTENT_GENERATION_CHANGED event
```

Low-level write tracing remains available through developer observability, not through the normal antivirus feed.

## 6. Race-free scanning by object and generation

Path-based scanning is vulnerable to rename/replacement races.

The security API should allow a privileged scanner to obtain a read-only handle to a specific object generation:

```text
OpenObjectGeneration(object_id, content_generation)
```

The returned handle must either:

- expose exactly that committed generation, or
- fail with `GENERATION_NOT_AVAILABLE`.

It must never silently return newer bytes under the same scan request.

The COW/checkpoint architecture may make short-lived stable scan views cheap.

## 7. Fast execution/open authorization

Real-time protection still needs a blocking path for sensitive operations such as execution.

The fast path should be:

```text
request execute object 0x1234 gen 87
      |
      +-- trusted scanner has valid verdict for gen 87 -> allow immediately
      |
      +-- no valid verdict -> request scan/authorization
```

The scanner should not be invoked synchronously for ordinary reads of known-clean unchanged content unless its policy explicitly requests that behavior.

Potential permission gates:

- execute
- load as executable/module/plugin
- open newly downloaded/untrusted content
- optional read/write gates for high-security products

This gating belongs to the OS/filesystem API layer, not to the AFS+ disk format.

## 8. Clone/reflink awareness

Reflinks create a major optimization opportunity.

If object B is a full clone of object A and initially represents identical content, a scanner should be able to reuse a content verdict rather than reread all shared extents.

The filesystem should expose clone/content-identity relationships without claiming that object identity is the same.

Example:

```text
A object=100 gen=9 content_identity=X
B object=101 gen=1 content_identity=X
```

A scanner that already trusts content identity X may immediately trust B under compatible metadata/origin policy.

If B later COW-modifies one byte, it receives a new content generation/identity and requires policy reevaluation.

`CloneRange` does not automatically imply whole-file identity and should not inherit a whole-file verdict unless the scanner explicitly supports that logic.

## 9. Optional content fingerprint cache

A rebuildable filesystem-maintained content fingerprint cache could further reduce repeated hashing by scanners, backup tools, build systems, and indexers.

Requirements:

- keyed to exact content generation
- algorithm explicitly identified
- invalidated atomically when content changes
- derived/rebuildable
- not treated as an antivirus verdict
- not writable by ordinary applications

Whether this is worth the CPU/write cost must be benchmarked before becoming a stable feature.

## 10. Security verdicts should not be ordinary file metadata

Do not store `clean=true` as a normal user-writable xattr.

Scanner verdicts depend on:

- engine version
- signature/database version
- policy
- origin/context
- trust state

The primary verdict cache should live in trusted security-service state keyed by filesystem/object/content generation.

A protected system metadata namespace may be useful for coordination, but it must not become an unverified source of truth.

## 11. Whole-volume initial scan without pathname overhead

A first scan should use:

```text
EnumerateObjects()
```

rather than recursive `opendir/stat/open` traversal.

The stream can return batches of:

- object ID
- type
- logical size
- content generation
- content fingerprint when present
- flags/policy/origin

The scanner opens data only for objects whose content actually needs inspection.

This is particularly important for volumes with millions of files.

## 12. Priority, filtering, and backpressure

Security consumers should be able to subscribe selectively:

```text
all content changes
executables only
new/untrusted files
files below/above size thresholds
specific policy roots
```

The persistent change stream provides correctness if an in-memory notification queue falls behind.

Notification overflow therefore does not force an immediate full-volume rescan as long as the persistent cursor history remains available.

## 13. Changed-range hints

For some scanners, knowing which byte ranges changed may reduce work.

AFS+ may optionally report changed ranges for a content generation when that information is naturally available from the transaction/COW engine.

This is a hint only.

The filesystem must never claim that scanning only changed ranges is equivalent to a full malware scan. The scanner decides whether the hint is safe for its format/signature model.

## 14. Security origin metadata

The filesystem should provide a protected extensible place for host security metadata such as:

- downloaded/untrusted origin
- quarantine state
- source application
- source URL/domain where the OS policy permits it

The on-disk core should not hard-code macOS quarantine or Windows Zone.Identifier semantics. Filesystem API v2 can expose a portable security-origin abstraction mapped to OS-specific metadata.

## 15. Resource target

The defining target is:

> Monitoring an idle or read-heavy filesystem whose content has not changed should be nearly free.

The persistent feed should scale with committed changes, not total opens or total namespace size.

Security benchmarks are specified separately and must measure CPU, RAM, event volume, added open/exec latency, data bytes reread, and performance under millions of unchanged files.

## 16. Broader value

Although antivirus is the motivating use case, this contract is also valuable to:

- backup software
- search/indexing
- DLP
- ransomware detection
- integrity monitoring
- package managers
- build caches
- sync engines

AFS+ should expose one excellent semantic content-change contract instead of adding a different ad-hoc hook for every product category.
