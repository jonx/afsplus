# ADR-066: Bounded open-unlinked lifecycle through a reserved orphan directory

Status: Accepted
Amends: ADR-039

<!-- toc -->

- [Context](#context)
- [Decision](#decision)
  - [Reserved internal directory](#reserved-internal-directory)
  - [Unlink and replacement](#unlink-and-replacement)
  - [Last close and crash recovery](#last-close-and-crash-recovery)
  - [Feature and compatibility class](#feature-and-compatibility-class)
- [Compatibility and resource analysis](#compatibility-and-resource-analysis)
- [Rejected alternatives](#rejected-alternatives)
- [Validation required for acceptance](#validation-required-for-acceptance)
- [Consequences](#consequences)

<!-- /toc -->

## Context

The filesystem-neutral VFS keeps file handles by stable object ID, but final
unlink immediately removes the object-map record and retires the file's
storage. An already-open handle therefore becomes stale, contrary to the
required open-unlinked semantics in the object model and crash test plan.

The Q3 low-space qualification also demonstrated why this cannot be repaired
only inside the handle table. A successful preallocation can leave a
checker-clean volume with too little raw free space for final unlink. Retiring
every extent of an arbitrarily fragmented file makes the unlink transaction's
reclaim metadata proportional to the file layout. A fixed reserve sized for
that transaction would be excessive on classic media and would still encode a
fragility limit.

After a crash, every process handle is gone. The filesystem must therefore be
able to locate every unlinked object without a full object-map scan, preserve
it while a live handle exists, and resume bounded cleanup when no handle can
exist. The state must also be intelligible to the checker and repair tools.

## Decision

### Reserved internal directory

Reserve object ID 2 as `OBJECT_ORPHAN_DIRECTORY`. Object IDs 3 through 15
remain reserved and dynamic IDs continue at 16. When present, object 2 is an
ordinary directory object whose AFST directory tree is not linked from the
user root. Its special reachability comes only from this feature and its fixed
object ID.

The directory is created lazily by the first transaction that needs to retain
an open file after its final user-visible link is removed. Once created, the
empty directory object and tree remain allocated. Avoiding repeated creation
and retirement keeps the low-space path predictable.

Each orphan directory entry:

- has a lowercase 16-digit hexadecimal name equal to its child object ID;
- uses the ordinary file child-type hint;
- points to exactly one regular file;
- is the file's sole directory reference, so the stored `link_count` remains
  1 and the existing object-record codec does not acquire a zero-link special
  case.

Object 2 and its entries are internal. Lookup, enumeration, stat by guessed
object ID, hard link and rename through the public filesystem API cannot
expose or manipulate them. Diagnostic and repair tools identify the directory
and its entries explicitly rather than pretending they are user namespace.

### Unlink and replacement

The VFS counts open file handles by object ID. Removing a non-final hard link
uses the ordinary decrement path. Removing the final user-visible link behaves
as follows:

- with no open handle, the existing final-delete transaction is permitted;
- with one or more open handles, one checkpoint transaction deletes the user
  directory entry and inserts the reserved orphan entry without retiring the
  object record, extent tree or data;
- atomic replacement applies the same rule to an open target before the source
  takes its visible name.

The object ID does not change. Reads, writes, truncate, data-policy operations
and fsync through an existing file handle continue to address that object.
Path lookup cannot rediscover it, and creating a new file with the removed name
creates a distinct object as usual.

The global intent-log window is checkpointed before the namespace transaction,
as for every immediate namespace operation. Orphan insertion/removal is not a
new intent-log operation in this ADR.

### Last close and crash recovery

When the last live handle closes, the VFS requests orphan cleanup. Failure to
complete cleanup never resurrects a name and never makes the close invalidate
the committed orphan state; the entry remains a restart point.

On a read-write mount, every persisted orphan is known to have no surviving
process handle. Cleanup can therefore resume from object 2 without scanning
the root namespace or the object map. A read-only mount leaves the entries
untouched and does not expose them.

Cleanup is restartable and bounded per checkpoint transaction:

1. remove no more than the configured number of logical extent records from
   the end of one orphan file, updating its object record and retiring or
   reference-dropping only those runs;
2. publish that smaller orphan file while its directory entry still exists;
3. once the file has no data or extent-tree blocks, delete its orphan entry and
   object record in a final small transaction;
4. process at most the configured object/extent budget before returning to the
   caller.

Every intermediate checkpoint is a valid orphan state. Repeating a step after
any crash is safe because the selected checkpoint contains either the old
layout or the shorter layout, never an external cursor that can advance ahead
of the object. Shared extents use ADR-061's normal reference-drop rules.

The normal mount path may locate object 2 with one bounded object-map point
lookup, but it does not synchronously drain an unbounded orphan set. Adapters
invoke bounded cleanup during mount/idle/sync and expose pending-orphan counts
through diagnostics. A mount is not allowed to degrade into a full-volume
scan.

### Feature and compatibility class

Allocate `RO_COMPAT` bit 1 to the permanent identity
`org.aros.afsplus:orphan-directory`. The bit permits object 2 to be an internal
object-map root and permits its file entries to be excluded from the user
namespace/reference count.

`RO_COMPAT` is required because an unaware writer would neither preserve the
lifecycle rules nor resume cleanup. It may safely read the visible namespace:
orphans have no visible names and ignoring them only withholds inaccessible
space. Aware readers and the portable C path accept the bit and ignore the
internal directory for ordinary lookup while diagnostic traversal labels it.

Formatting profiles that promise writable filesystem semantics enable the
feature. An older volume without the bit keeps the immediate-final-delete
behavior and does not advertise the open-unlinked capability; the reserved
object ID is otherwise absent.

No checkpoint field, new tree kind or object-record layout is introduced.
The encoding reuses the typed directory and object-map formats already
implemented by both reference paths.

## Compatibility and resource analysis

The first orphan insertion can allocate the fixed directory object/tree in
addition to two bounded directory mutations and object-record/object-map COW.
Subsequent unlink transactions mutate an existing tree. Their allocation cost
depends on tree height, not file size or extent count.

Each cleanup transaction has an explicit extent and object budget. Classic
profiles choose a small budget and modern profiles may choose a larger one;
the on-disk representation is identical. The implementation reports the
chosen budget and peak transaction/cache memory in qualification output.

Because the feature bit is set at format time, an enabled-but-unused volume can
force an unaware implementation to read-only mode. This is accepted for
writable profiles: silently losing open-unlinked crash recovery is worse than
the extra compatibility restriction. Read-only/minimal profiles may omit the
feature when they never promise writable handle semantics.

## Rejected alternatives

- **Keep an in-memory tombstone and delay namespace unlink until close.** Other
  lookups would continue to see the name, fsync could not make the unlink
  durable, and a crash would lose the lifecycle state.
- **Keep `link_count == 0` objects only in the object map.** Recovery would
  require a full object-map scan, violating bounded mount, and the checker
  could not distinguish an intentional orphan from a leaked object without an
  additional authority.
- **Add an orphan-tree root to the checkpoint.** It adds a new checkpoint
  payload and tree wire surface although a fixed reserved object can already
  be found by bounded point lookup and reuse the validated directory codec.
- **Use a visible `.afsplus-orphans` directory under the user root.** Older
  readers would expose and permit mutation of internal lifecycle state;
  namespace spelling is not an integrity boundary.
- **Retire the complete file in the unlink transaction and reserve for the
  worst case.** The number of fragmented extents and reclaim-segment writes
  can scale with the file, so the reserve becomes a capacity tax or a hidden
  operational limit.
- **Write orphan IDs into the reclaim queue.** That queue owns physical runs
  after their last logical reference; an open orphan is the opposite — a live
  object whose mappings must remain readable and writable. Combining the two
  would make both validators ambiguous.

## Validation required for acceptance

- format constants, feature registry/profiles, public C header and Rust/C
  negotiation agree on reserved object ID 2 and `RO_COMPAT` bit 1;
- formatter, mount, exhaustive checker and dump accept the absent, empty and
  populated orphan-directory states and reject malformed names, wrong child
  types, duplicate visible references, missing feature bits and any public
  namespace reference to object 2;
- handle tests cover read, write, truncate and fsync after unlink, multiple
  handles, last-close cleanup, hard links, name reuse and open-target atomic
  replacement;
- every write/flush cut of first orphan-directory creation, ordinary orphan
  insertion, post-unlink file update and each cleanup step recovers to an
  allowed semantic state and passes the checker;
- a multi-extent orphan cleanup proves the per-transaction budget, resumes
  across remount, and eventually returns all private capacity after the normal
  checkpoint quarantine;
- low-space qualification proves visible unlink can commit using bounded
  emergency headroom independent of file extent count, then distinguishes raw
  free from normally available space;
- Hosted MacAROS and host adapters exercise the same VFS behavior; the
  portable C reader and m68k compile/fuzz gates remain green.

## Consequences

AFS+ gains POSIX-like open-unlinked identity without adding a third lifetime
counter to object records or a new checkpoint root. The same mechanism makes
visible unlink a bounded namespace transaction, allowing Q3's emergency
metadata reserve to be sized for structural paths rather than arbitrary file
fragmentation. The cost is one reserved object identity, one `RO_COMPAT`
feature bit, new checker special cases and restartable cleanup code that must
be qualified before M03 or Q3 can close.
