# ADR-030: First-class content inspection contract

Status: Proposed

## Context

Antivirus and security products traditionally monitor files by intercepting broad filesystem I/O or recursively scanning paths. Existing systems such as Windows minifilters, Linux fanotify, and macOS Endpoint Security provide powerful hooks, but naïve use can impose overhead on large numbers of opens and reads.

AFS+ already plans stable object IDs, content generations, persistent change streams, global enumeration, reflinks, and checkpointed COW metadata. Together these make a lower-overhead content-inspection model possible.

## Proposed decision

Filesystem API v2 should expose a privileged, filesystem-neutral content-inspection contract built around:

- stable filesystem UUID + object ID
- monotonically changing content generation
- durable change-stream cursors
- whole-filesystem batch enumeration
- race-free read access to a specific committed object generation where available
- optional authorization gates for security-sensitive accesses such as execution
- clone/content-identity information for verdict reuse
- structured origin/security metadata

The filesystem does not implement malware detection and does not store an ordinary user-writable `clean` verdict.

## Performance principle

The common fast path is `scan once per content generation`.

Unchanged content should not be rescanned merely because it was reopened, renamed, or read again.

Security event generation should normally be content-generation/transaction oriented rather than one event per write syscall.

## Correctness principle

Persistent event history, not a finite in-memory notification queue, is the source of catch-up correctness.

If history has expired, the consumer receives `RESCAN_REQUIRED` and falls back to `EnumerateObjects()`.

## Disk-format impact

Epoch 1 must provide an unambiguous content-generation concept for regular-file data.

The remainder of the security subscription, authorization, and scanner-verdict machinery belongs to Filesystem API v2 / OS security services rather than to the core on-disk format.

Any optional content fingerprint accelerator remains derived/rebuildable and requires a separate feature decision.

## Consequences

AFS+ can support antivirus, backup, indexing, DLP, ransomware detection, and integrity monitoring through a common low-overhead semantic interface.

Security benchmarks must become part of the qualification suite before claiming that this design improves real-world resource usage.
