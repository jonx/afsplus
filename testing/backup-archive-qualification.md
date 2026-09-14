# Backup archive qualification

> **ADRs:** [ADR-076](../adr/ADR-076-pax-backup-interchange.md), [ADR-078](../adr/ADR-078-backup-preservation-modes.md) · **Spec:** none ·
> **Tests:** none · **Milestones:** M13, M14

<!-- toc -->

- [PAX record codec](#pax-record-codec)
- [Tar framing and independent recovery](#tar-framing-and-independent-recovery)
- [Completion envelope](#completion-envelope)
- [Integration acceptance](#integration-acceptance)
- [Effective member admission](#effective-member-admission)
- [Streaming local-record binding](#streaming-local-record-binding)
- [Object metadata admission](#object-metadata-admission)
- [Opaque values through an authorized archive consumer](#opaque-values-through-an-authorized-archive-consumer)
- [Verified scratch replay](#verified-scratch-replay)
- [Complete object inventory groups](#complete-object-inventory-groups)
- [Sparse content consumer](#sparse-content-consumer)
- [Allocation-preserving consumer](#allocation-preserving-consumer)
- [Bound regular-file groups](#bound-regular-file-groups)

<!-- /toc -->

## PAX record codec

Run `cargo test -p afsplus-backup` for the bounded extended-header codec in
[pax.rs](../crates/afsplus-backup/src/pax.rs). The
[Oracle pax manual](https://docs.oracle.com/cd/E88353_01/html/E37839/pax-1.html)
specifies decimal record lengths in bytes, including the trailing newline,
and UTF-8 encoding. Values can contain embedded newlines and equals signs;
record boundaries follow the declared lengths. The codec preserves empty values
for interpretation by the profile layer.

Require exact equality with the independent Python tarfile fixture containing
UTF-8, a newline/equal-bearing path and a negative fractional timestamp. Exercise
decimal-length width transitions, every nonempty truncated prefix of each
single-record fixture, invalid UTF-8, embedded NUL, malformed/overflowing lengths,
duplicate keys, trailing garbage and deterministic arbitrary bytes.

The codec contract uses canonical positive decimal lengths and unique keys in
a record block. Reject duplicate keys before any profile interpretation. General
PAX global/local override and alternate character-set handling require a separate
archive-layer decision. Keywords containing NUL, newline or equals sign are
invalid under this strict codec. Values require UTF-8 and exclude NUL.

Callers supply positive byte, record and key limits; a zero value-byte limit
allows only empty values. Admission precedes output allocation. Decoding borrows
keys and values from the input and allocates record-index bookkeeping within the
record count; encoding preflights the complete output byte size. Test every
limit independently in both directions, including exact-boundary admission.
Caller buffer limits, process peak RAM and target runtime need separate evidence.

## Tar framing and independent recovery

Run `cargo test -p afsplus-backup` and
[check-backup-tar.sh](../tools/check-backup-tar.sh). The host gate requires
Python tarfile and bsdtar. It regenerates the retained Python ustar fixture,
recovers the Rust writer's exact ordinary-file bytes with both independent
readers, compares Python-visible header metadata, and requires a changed-payload
negative control to fail its byte oracle. Extraction streams to a private
temporary file; it does not apply archive paths to a host directory.

The [framing module](../crates/afsplus-backup/src/tar.rs) validates the unsigned
header checksum, ustar magic/version, octal numeric fields, fixed UTF-8 name
fields, supported member types, reserved bytes and zero payload padding.
It supports files, directories, links and PAX local/global headers. Unsupported
numeric/header encodings and member types are explicit errors. Long names use
the ustar prefix when representable; the profile layer supplies PAX names and
nonrepresentable numbers through its validated metadata rules.

A reader requires `begin_payload` after each header, selecting its raw size or
an independently validated PAX size override for a regular file. Selection
precedes payload I/O and respects the caller's member-byte bound. Stream payloads
through caller buffers, drain the exact admitted length and check padding before
advancing. An unfinished member returns busy. Errors after consuming stream
bytes poison the stream; subsequent operations cannot resume ambiguously.

Require two zero end blocks and actual EOF, admitting only a configured number
of extra complete zero blocks. Bound the member count independently of member
bytes. The writer preflights a header and size before output, refuses excessive
payload input, and reports partial-write/padding/end-marker/flush errors.
Successful finish establishes framing; destination durability and profile
completion need their own validation.

Tests cover the Python fixture, every truncated prefix of the Rust fixture,
header/padding/end-marker damage, valid-checksum malformed fields, long UTF-8
names, unsupported types, numeric boundaries, member/byte/padding limits,
payload sequencing, maximum 64-bit override arithmetic and partial I/O errors.
A giant declared length test checks arithmetic/state admission only. Full large
and sparse streams require separate workload evidence. Header buffers are fixed
at 512 bytes; name allocations are bounded by field widths. Total runtime RAM
and constrained-target execution require separate measurements.

## Completion envelope

[ADR-081](../adr/ADR-081-ordinary-pax-completion-member.md) and the
[envelope specification](../spec/backup-envelope.md) define version 2.
Run `cargo test -p afsplus-backup` and
[check-backup-envelope.sh](../tools/check-backup-envelope.sh). The host gate uses
OpenSSL with SHA-512/256 support for independent digest verification, Python for
raw tar/count checks, and Python/bsdtar for ordinary file recovery. Set
`AFSPLUS_OPENSSL` for an explicit executable; the script recognizes the existing
Homebrew OpenSSL 3 path on macOS. This executable is a test dependency.

Require exact receipt equality, delayed receipt availability, changed body/header
rejection, every truncated fixture prefix, missing/duplicate/altered terminal
records, wrong counts, unsupported controls, trailing members, unfinished writes
and flush failures. Validate body count/byte limits separately from the bounded
control payloads. Raw body paths reject control collisions and parent traversal;
resolved PAX names and symlink-safe destination behavior require profile checks.

Verify identical digests with the compact portable hash backend by running the
crate tests and independent gate with:

```sh
RUSTFLAGS='--cfg sha2_backend="soft" --cfg sha2_backend_soft="compact"' cargo test -p afsplus-backup
RUSTFLAGS='--cfg sha2_backend="soft" --cfg sha2_backend_soft="compact"' tools/check-backup-envelope.sh
```

[RustCrypto sha2](https://docs.rs/crate/sha2/0.11.0) defines these backend
configuration flags. Keep software/default build artifacts separate during
concurrent qualification. The hash state is incremental; input length is counted
in 128 bits and admitted within the standard's domain. Tests at the maximum
counter validate refusal without output, independently of huge-file workloads.
An integrity receipt provides no sender authentication and cannot replace the
preservation profile or host grants. Native/older CPU execution, peak RAM,
throughput and sustained large streams need separate measurements.

## Integration acceptance

Record decoding establishes syntax only. The complete consumer must enforce the
versioned preservation profile, path confinement, required metadata, payload
integrity and a completion record before declaring a complete backup/restore.
A valid sequence of PAX records or a valid tar prefix cannot establish completion.

Qualify snapshot capture and destination-scoped restoration, independent ordinary
file recovery, sparse layouts, hard-link identity, timestamps, attributes,
security metadata, reservations, revocation, truncated output, conflicting
metadata, unknown requirements and resource pressure under ADR-076/078.
Every full-preservation omission requires refusal; explicit content recovery
reports the losses allowed by its profile. Profile identity,
integrity and completion semantics follow the format decision process before
normative archive fixtures are generated.

## Effective member admission

Run `cargo test -p afsplus-backup member::tests` for effective local-PAX field
validation. The oracle supplies a safe raw header and malicious overriding
paths, verifies hard-link containment and symlink data semantics, exercises
unknown-field refusal, duplicate records and explicit byte budgets, and checks
64-bit numeric boundaries. Exact signed timestamp cases cover nanoseconds,
negative fractions and both signed-second limits without floating-point
conversion. Invalid precision is refused, never rounded.

The [member contract](../spec/backup-envelope.md#effective-ordinary-member-fields)
is an admission prerequisite for the preservation consumer. Its tests do not
qualify sparse/security transport, archive-wide identity or actual restoration.

The [Oracle PAX reference](https://docs.oracle.com/cd/E88353_01/html/E37839/pax-1.html)
describes field overrides and decimal subsecond units. The preservation
interface deliberately refuses precision loss and retains identity fields
without looking up host accounts.

## Streaming local-record binding

Run `cargo test -p afsplus-backup stream::tests`. The integrated reader oracle
verifies that local path, size and signed-time overrides apply exactly once,
that raw placeholder size does not misframe payload bytes, and that reads use
caller-sized buffers. It verifies retryable busy advancement, metadata admission
before buffer allocation, permanent failure for dangling/stacked/escaping local
records, and absent receipts for every truncated archive prefix. Completion is
checked separately from member consumption and cannot hide a dangling local
record even if the underlying envelope digest is valid.

## Object metadata admission

Run `cargo test -p afsplus-backup metadata::tests` against the
[object metadata contract](../spec/backup-object-metadata.md) selected by
[ADR-082](../adr/ADR-082-backup-object-metadata.md). Compare exact protection,
three signed nanosecond timestamps, source path and kind. Exercise all nine
combinations of attribute/security inventory knowledge and reordered fields.
Every required field must fail when omitted. Unknown versions/keys/kinds,
duplicate fields, malformed scalars, escaping paths and every truncated prefix
must fail; exact byte admission applies to encoding and decoding. A decoded
string must borrow the admitted input. Archive-wide matching, lossless inventory
transport and authorized restoration are separate integration requirements.

## Opaque values through an authorized archive consumer

Run `cargo test -p afsplus-backup --all-features`. The optional `consumer`
feature links the filesystem-neutral VFS interfaces; standalone codecs retain
their independent dependency profile. [ADR-084](../adr/ADR-084-opaque-backup-value-pairs.md)
and the [value-pair specification](../spec/backup-opaque-values.md) define the
transport. Test source snapshot capture, descriptor enumeration, two-byte export,
archive framing and three-byte staged import with exact key/encoding/binary
comparisons. Include empty values and live source mutation after capture.

For an unverified input stream, publication requires the original reader's
verified digest and EOF. Reject a
matching receipt from another reader, premature publication, revoked destination
authority, wrong source binding and changed payload bytes. Every truncated
archive prefix must leave active destination metadata absent and release staging.
Short/long source values and revoked source reads must poison writer completion.
Descriptor tests cover each missing field, unknown versions/keys/classes,
malformed numeric fields, duplicates, budgets and every truncated payload.

These fixtures qualify one opaque-value transport component through independent
in-memory providers. Complete object/inventory matching, unique keys/ordinals,
large-inventory staging/spooling, sparse data, AFS+ opaque storage, generic-tool
metadata recovery and native resource/durability behavior need separate gates.

## Verified scratch replay

Run `cargo test -p afsplus-backup --all-features` for
[verified scratch](../spec/backup-spool.md) under
[ADR-085](../adr/ADR-085-verified-archive-spooling.md). Compare the three-leaf root
with Python hashlib using an independent recursive tree split. Exercise odd tree
sizes and partial chunks, modified and locally resealed data, reordered chunks,
changed proof hashes, nonzero padding and truncated storage. A failed chunk must
leave the caller buffer unchanged and permanently fail replay.

Capture must refuse invalid quotas, malformed/truncated archives and scratch
read/write/seek/flush errors. Exact archive/scratch admission boundaries must
succeed. A temporary regular file must replay through final completion and match
its reported scratch size. `scratch_overhead_and_read_requests_are_measured`
prints chunk size, archive/scratch bytes, levels and read/write counts/bytes;
maximum scratch read request must not exceed one chunk.

The integrated consumer must restore 20 distinct keys with only root plus one
upload slot, releasing staging after each publication before replay EOF. A replay
failure must withdraw early-publication admission. Full inventory validation,
segmented large-archive storage, sustained workload measurements and native
provider lifecycle tests have separate completion gates.


## Complete object inventory groups

Run `cargo test -p afsplus-backup --all-features` for
[inventory manifests](../spec/backup-inventory.md) under
[ADR-086](../adr/ADR-086-backup-inventory-manifests.md). Round-trip empty,
single-entry and 70-entry groups using pages of 1, 3 and 64 entries, one-byte
transfer buffers, a 512-byte spool chunk and root plus one upload slot.
Compare every restored key and byte, summaries and next ordinals. Descriptor
page calls must equal two complete enumerations, independent of value size.

Refuse uninspected sources, second-pass encoding changes with equal count/size,
entry/byte/page/ordinal limits, malformed or incomplete manifest fields,
noncanonical integers, duplicate keys, mismatched hashes/counts/sizes/classes,
wrong paths, unverified replay and revoked destination authority. Failures must
return no success summary, poison further archive use and release every upload;
earlier complete publications are a documented partial outcome. Check empty
manifests at the final ordinal and checked arithmetic overflow.

These are hosted component fixtures, not complete backup-job or older-machine
qualification. Cross-object completeness, data/namespace/reservation binding,
content-recovery loss reports and native resource/durability evidence belong to
the enclosing archive and platform gates.


## Sparse content consumer

Run `cargo test -p afsplus-backup --all-features` and
[check-backup-sparse.sh](../tools/check-backup-sparse.sh) for
[sparse transport](../spec/backup-sparse.md),
[ADR-087](../adr/ADR-087-sparse-archive-content.md) and
[ADR-088](../adr/ADR-088-sparse-stored-size-field.md).

The real-service fixture captures a file with written data, written zeros,
unwritten ranges inside and beyond EOF, and a 1 TiB-plus logical length. Modify
and shrink the live source after capture, then export through a remounted view.
Require an archive below 16 KiB and fewer than 256 source block reads, with zero
source writes/barriers. Restore into a scoped empty AFS+ file, synchronize,
remount and compare captured bytes, sampled logical holes, exact logical size,
written allocation and an untouched outside file. The content report must name
both omitted reservation ranges and their total bytes; no full-preservation
claim follows from their omission.

Exercise one-entry allocation pages, caller transfer buffers, revoked source
authority during output (including all-hole files), wrong destination paths,
revoked destination grants and nonempty destination refusal. Failure must poison
further archive use. Compare maps across multiple 512-byte blocks, empty maps,
EOF markers and maximum unsigned logical length. Refuse malformed numbers,
overlap, out-of-file and overflowing ranges, interior zero-length entries,
nonzero padding, wrong stored lengths and each map/data/logical/count quota.
Reject impossible map counts before allocation. Every truncated archive prefix
must withhold completion. Default stream/spool constructors must refuse sparse
headers; dedicated constructors must expose separate lengths and require map
validation before writes.

Python and libarchive must recover mixed, all-hole and empty members, preserving
contents and following-member alignment. The all-hole extraction must not
allocate its whole logical length. Python's independent numeric codec must match
raw headers at the octal boundary and unsigned 64-bit maximum. Keep conflicting
PAX `size` as a parser refusal regression. Header tests do not qualify a giant
stored payload. Independent generic extraction is content recovery only; full
metadata, reservation and complete-job oracles retain separate gates.

## Allocation-preserving consumer

Run `cargo test -p afsplus-backup --all-features` for
[allocation preservation](../spec/backup-allocation.md) and
[ADR-090](../adr/ADR-090-archive-allocation-preservation.md).

Export remounted captured files of logical length zero, 1 TiB plus 23 bytes and
u64 maximum, with written data/zeros and unwritten reservations inside/beyond
EOF and in the final address block. Mutate the live source after capture. Use
one-entry pages and 4 KiB transfers/reservation calls; require an archive below
16 KiB, exact reservation-loss counts and exhausted ordinal reporting at the
final member. Restore each archive in both modes, synchronize and remount.
Compare logical size, bytes, sampled holes and a separately constructed logical
block/state oracle, including the final block. Recovery must omit precisely the
reported reservation capacity.

Construct a second archive independently of the source planner with written
allocation rounded past EOF. Wrong source path, logical size, written state,
sparse map or raw ordinal must fail before content/reservation writes. Exercise
revoked grants, ordinal exhaustion and zero/insufficient reservation, page and
readback budgets. Failure poisons further archive use. Destination providers
with unsupported readback or reservations must fail preservation before content
writes while permitting explicitly selected recovery with a loss report. Move
a reservation without changing its size and require verification failure after
the partial writes. Split each destination extent into equivalent adjacent byte
ranges and require success. Exhaust a nonzero readback budget after writes and
withhold success.

Codec tests cover empty and final-address records, every truncated prefix,
unknown/missing/duplicate fields, noncanonical numbers, invalid states,
ordering/overlap/end overflow and byte/count/logical limits. Compare interval
coverage across splitting/merging and the `2^64` endpoint; changed holes, flags,
missing and excess coverage must fail.

This component gate does not certify complete object/namespace preservation,
AFS+ opaque metadata storage, job-level loss-report persistence, sustained large
payloads, spooled maps, older-machine peak memory or native durability. Preserve
those gates in the enclosing backup and platform qualification.

## Bound regular-file groups

Run `cargo test -p afsplus-backup --all-features` for
[regular-file groups](../spec/backup-file.md) and
[ADR-091](../adr/ADR-091-bound-regular-file-archive-groups.md).

The real AFS+ fixture sets distinct protection and nanosecond timestamps, captures
a sparse file with reservations, changes its live bytes and exports recovery
from a remounted view. Use a 512-byte transfer buffer and one-entry pages. Restore,
consume EOF, synchronize and remount; compare exact captured bytes, size,
protection and all timestamps, and confirm reservations are omitted with the
correct loss count/bytes. Attribute and security knowledge must be explicitly
Uninspected rather than asserted empty. Exercise the final available group
ordinals and require explicit exhaustion.

A separate semantic provider fixture covers full inventory transport, including
unknown binary security bytes, signed timestamp extremes and full-width
protection. One-byte transfers and one-entry pages must preserve the file and
opaque values without hole expansion. Recovery of a full group must validate
its inventory counts/bytes and report them as discarded while making zero opaque
publications, even when the provider refuses uploads. Recovery groups retain
Uninspected knowledge and no fabricated transported summary. Full restore must
refuse a recovery group before writes.

Rebuild valid integrity envelopes containing conflicting file paths, kinds,
mtime, inventory knowledge, descriptor hashes or value paths. Unknown group
versions also fail. Check which failures precede file writes, and withhold core
metadata finalization after inventory failure. A provider that rounds timestamps
must not yield a file-success report. Revoke destination authority and exhaust
inventory counts/pages, buffers and mode consistency; require sticky failure and
release of staged uploads. Source revocation, uninspected full-export inventory,
second-pass descriptor mutation and ordinal exhaustion must poison export.

The real AFS+ recovery fixture and the semantic full-inventory provider prove
different scopes. They do not establish AFS+ opaque storage support, directories,
symlinks, hard-link identity, whole-namespace completeness, durable job loss
reports, sustained resource use or native/older-system qualification. Preserve
those acceptance gates explicitly.
