# ADR-068: Store symbolic-link targets inline as opaque UTF-8

Status: Accepted
Amended by: ADR-094
Amends: ADR-039

<!-- toc -->

- [Context](#context)
- [Decision](#decision)
  - [Object and target encoding](#object-and-target-encoding)
  - [Namespace and object lifetime](#namespace-and-object-lifetime)
  - [API and path interpretation](#api-and-path-interpretation)
  - [Compatibility and profiles](#compatibility-and-profiles)
- [Compatibility and resource analysis](#compatibility-and-resource-analysis)
- [Rejected alternatives](#rejected-alternatives)
- [Validation required for acceptance](#validation-required-for-acceptance)
- [Consequences](#consequences)

<!-- /toc -->

## Context

AFS+ already reserves object type 3 for symbolic links, the filesystem API
lists `symlink` and `readlink`, and the host and AROS adapters can report a
symlink node kind. The object codec nevertheless rejects the type and the
directory codec rejects its child-type hint. No target representation or
cross-platform interpretation contract has been selected.

A symlink target is security-sensitive metadata: silent damage can redirect a
path even when the referenced file data is intact. Storing it in an ordinary
unchecked data extent would add allocation and reclamation work, make a small
namespace object span two independently updated blocks, and leave the target
without the metadata checksum used by the object record.

The target text also cannot be normalized into one invented universal path
syntax without changing established host semantics. POSIX absolute paths,
AROS volume prefixes, Assigns and `PROGDIR:` are namespace concepts. A mounted
filesystem normally returns the exact target text to its OS path resolver.
Relative component paths are naturally portable; namespace-qualified targets
may intentionally be meaningful only on the system that created them.

## Decision

### Object and target encoding

Object type 3 is the base-format symbolic-link type. A symlink uses the
existing checksummed `AFSO` block and fixed 96-byte object-record prefix. Its
target bytes immediately follow that prefix:

```text
object payload length = 96 + target byte length
payload[96..]         = target bytes
size_bytes            = target byte length
allocated_bytes       = 0
data_root              = 0
data_blocks            = 0
flags                  = 0
```

The target is non-empty, valid UTF-8 and contains no NUL byte. It is otherwise
opaque: bytes such as `/`, `:`, repeated separators, `.` and `..` are stored
and returned unchanged. Its maximum is
`block_size - common_header_size - 96`, which is 3,968 bytes for the current
4 KiB block format. The API reports this limit; encoders use checked
arithmetic before copying any bytes.

The common block CRC covers the fixed fields, target and zeroed tail. A
decoder requires the payload length to equal `96 + size_bytes`; it rejects
truncation, trailing payload bytes, invalid UTF-8, NUL, nonzero allocation
fields or flags, and inconsistent object/header identity before exposing the
target.

This is an inline payload, not the optional tiny-file mechanism. Regular-file
inline data remains a separate open question and cannot infer semantics from
the symlink encoding.

### Namespace and object lifetime

A directory entry pointing to a symlink uses child-type hint 3. The object
record remains authoritative and the checker requires the hint to match it.
Creation publishes the new object record, object-map entry and parent
directory entry in one checkpoint transaction. Rename preserves object ID and
target bytes. Unlink removes the directory entry and symlink object in one
bounded transaction; there are no data extents and symlinks do not enter the
open-file orphan lifecycle.

The first implementation creates symlinks with `link_count == 1` and rejects
hard-link creation for them. The wire invariant remains `link_count >= 1`, so
supporting hard links to symlink objects later does not require a record
layout change. A target is immutable for the object's lifetime: changing it
means atomically replacing the directory entry with a newly created symlink
object, matching ordinary host semantics.

### API and path interpretation

Filesystem API v2 adds a `SYMLINKS` capability and two bounded operations:

- `CreateSymlink(parent_id, name, target, metadata)` returns the new stable
  object ID; and
- `ReadLink(object_id, caller_buffer)` returns the exact stored target or the
  required byte count without hidden allocation.

Neither the format codec nor the object-ID-based VFS follows the target.
Loop detection, maximum follow count, relative-base selection, AROS Assign
resolution and POSIX mount-namespace behavior belong to the calling path
layer. The portable conformance target is exact preservation: a relative
UTF-8 target created on one implementation is read byte-for-byte by all
others. Namespace-specific targets must also round-trip, but are not promised
to resolve on a different OS.

The Rust VFS, portable C API, FUSE adapter, AROS Rust adapter and diagnostic
tools expose the same bytes. Text and JSON tools escape control characters so
a target cannot forge a diagnostic line or JSON field.

### Compatibility and profiles

No feature bit is allocated. Symbolic link object type 3 and directory hint 3
were already reserved as base-format identities, and symlinks are an M03
epoch-1 requirement rather than an optional disk accelerator. Before the M14
wire freeze, older experimental implementations already reject this object
type or hint instead of silently modifying it.

An implementation may omit symlink creation on a constrained or read-only
profile and leave `SYMLINKS` clear, but an epoch-1 reader must validate,
enumerate and return an existing target through its bounded read API. A
read-write implementation that cannot preserve the base symlink object must
refuse the mount rather than advertise partial support.

## Compatibility and resource analysis

A symlink adds one object-record block plus the ordinary object-map and
directory COW paths. Reading it needs one bounded object lookup and one object
block; it allocates no data buffer when the caller supplies the destination.
The format maximum is derived from block size, so classic systems never need a
`PATH_MAX`-sized heap allocation. They may return the required size and let the
caller retry with an adequate buffer.

Using opaque UTF-8 preserves POSIX and AROS namespace behavior and avoids
embedding either OS grammar in the volume. A relative target composed of AFS+
component names is the portable interchange subset. An AROS Assign or POSIX
absolute target remains valid stored data but can be dangling or mean
something different after moving the volume, as it can when moving media
between two machines of the same OS.

## Rejected alternatives

- **Store the target in an ordinary data extent.** This costs another block,
  reclaim state and I/O for a small metadata object, and ordinary file data is
  not protected by the metadata-block CRC.
- **Reuse the regular-file inline-data feature.** It couples mandatory
  symlinks to an optional tiny-file policy and leaves type-specific invariants
  ambiguous.
- **Allocate a dedicated symlink-target block type.** It retains checksums but
  still adds a second block and transaction dependency without increasing the
  useful target limit at the current block size.
- **Store a normalized universal path.** Translating AROS Assigns or POSIX
  absolute paths changes `readlink` bytes and cannot preserve both namespace
  models. Resolution stays outside the disk format.
- **Store arbitrary bytes.** AFS+ names are UTF-8 and all target systems need a
  bounded interoperable representation. NUL-free UTF-8 gives C, Rust, AROS
  and host adapters one exact contract.
- **Allocate an optional feature bit.** Epoch-1 symlink reading is baseline
  functionality, while implementations already fail closed on the reserved
  type during the experimental period.

## Validation required for acceptance

- Rust and independent C codecs agree on every field, maximum target length,
  exact bytes and error class for malformed lengths, UTF-8, NUL, flags and
  allocation fields;
- directory, object-map, checker and dump paths accept hint/type 3 and reject
  hint mismatches, unreachable records, wrong link counts and forged valid-CRC
  targets;
- create, readlink, rename, final unlink and atomic replacement preserve the
  exact target and stable-ID rules through the Rust VFS and adapters;
- every modeled write/flush cut of create, unlink and replacement selects the
  complete before or after namespace and passes the exhaustive checker;
- tests cover dangling, relative, absolute, AROS-qualified, Unicode,
  `.`/`..`, maximum-length and too-long targets, plus caller-buffer retry;
- FUSE and Hosted MacAROS cross-read the same fixture, and portable C passes
  strict, sanitizer, static-analysis, fuzz and configured m68k compile gates;
  and
- tool output escapes target bytes deterministically in text and schema-v1
  JSON.

## Consequences

Symlinks become a small, checksummed, one-record namespace object with bounded
classic-system memory use and exact cross-implementation round-trip. The core
format does not pretend that POSIX and AROS have the same namespace.

The main cost is a 3,968-byte target ceiling at 4 KiB blocks and a variable
object payload for one object type. Longer links would require a future
negotiated representation. Every epoch-1 reader must understand symlink
records even if its writable profile does not expose creation.
