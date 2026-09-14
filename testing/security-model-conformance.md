# Security Model Conformance and Performance Tests

> **ADRs:** [ADR-031](../adr/ADR-031-portable-security-acls.md), [ADR-075](../adr/ADR-075-revocable-backup-capability.md) · **Spec:** [security model](../docs/30-portable-security-model.md), [backup API](../docs/13-filesystem-api-v2.md#8-trusted-snapshot-backup-extension) ·
> **Tests:** [backup harness](../crates/afsplus-vfs/tests/backup.rs) · **Milestones:** M14

<!-- toc -->

- [1. Identity round-trip](#1-identity-round-trip)
- [2. ACL evaluation vectors](#2-acl-evaluation-vectors)
- [3. Inheritance vectors](#3-inheritance-vectors)
- [4. Cross-platform projection](#4-cross-platform-projection)
  - [POSIX adapter](#posix-adapter)
  - [Windows adapter](#windows-adapter)
  - [Classic Amiga/AROS adapter](#classic-amigaaros-adapter)
- [5. Strict mount behavior](#5-strict-mount-behavior)
- [6. Security descriptor sharing](#6-security-descriptor-sharing)
- [7. Security-domain prototype](#7-security-domain-prototype)
- [8. Access-check performance](#8-access-check-performance)
- [9. Namespace operation security semantics](#9-namespace-operation-security-semantics)
- [10. Raw-media threat test](#10-raw-media-threat-test)
- [11. Failure injection](#11-failure-injection)
- [12. Benchmark reporting](#12-benchmark-reporting)
- [13. Trusted snapshot backup authority](#13-trusted-snapshot-backup-authority)
- [14. Destination restore authority](#14-destination-restore-authority)
- [15. Captured allocation enumeration](#15-captured-allocation-enumeration)
- [16. Destination reservation restoration](#16-destination-reservation-restoration)
- [17. Captured metadata inventory knowledge](#17-captured-metadata-inventory-knowledge)
- [18. Opaque captured metadata transport](#18-opaque-captured-metadata-transport)
- [19. Staged opaque destination metadata](#19-staged-opaque-destination-metadata)
- [20. Destination allocation readback](#20-destination-allocation-readback)

<!-- /toc -->

## 1. Identity round-trip

Verify that stable AFS+ principals survive moves between hosts even when the current host cannot resolve the account.

Cases:

- known local user
- known group
- unmapped foreign user
- unmapped foreign group
- OWNER@ / GROUP@ / EVERYONE@
- deleted local account whose AFS+ principal remains on disk

No test may silently replace an unknown principal with another principal.

## 2. ACL evaluation vectors

Build language-neutral test vectors covering:

- ALLOW only
- DENY before ALLOW
- overlapping user/group memberships
- EVERYONE@ interaction
- owner and owning-group interaction
- READ/WRITE/EXECUTE/TRAVERSE
- DELETE vs DELETE_CHILD
- ACL-management rights

Rust and C implementations must produce identical granted/denied masks.

## 3. Inheritance vectors

Test every combination of:

- file inherit
- directory inherit
- inherit only
- no propagate
- inherited marker
- protected/stop-inheritance descriptor

Include creation, rename, move, copy, reflink clone, and hard-link operations.

## 4. Cross-platform projection

### POSIX adapter

Test:

- simple rwx lossless projection
- named user/group POSIX ACL projection
- ACL containing DENY that POSIX cannot express
- inheritance combinations POSIX cannot express

Expected behavior must distinguish lossless, preserved-but-not-exposed, and degraded mappings.

### Windows adapter

Test canonical ALLOW/DENY, owner, inheritance and protected ACL round trips against equivalent Windows security descriptors where possible.

### Classic Amiga/AROS adapter

Test that protection-bit edits do not accidentally erase richer security descriptors.

## 5. Strict mount behavior

A host that cannot enforce an active security feature must be tested in:

- strict mode: unsafe RW mount/operation rejected
- preserve mode: full metadata retained while the AFS+ layer continues authoritative enforcement where possible
- compat mode: downgrade only after explicit opt-in

## 6. Security descriptor sharing

Create one million files inheriting the same descriptor and measure:

- on-disk metadata overhead
- descriptor lookup CPU
- cache memory
- create latency
- ACL-change cost

Then compare against duplicated per-object ACL storage.

Shared descriptors must behave immutably: modifying one object's ACL must not alter another object's effective ACL unintentionally.

## 7. Security-domain prototype

If security domains are implemented, benchmark:

- policy change over 1 million-object subtree
- normal access-check cost
- cache hit/miss cost
- move between domains
- hard-link constraints across domains
- descriptor override density

Compare against recursively materialized ACL changes.

## 8. Access-check performance

Benchmark hot-path access checks for:

- single-user/classic profile
- simple owner/group/everyone ACL
- 8 ACE ACL
- 32 ACE ACL
- 128 ACE pathological ACL
- many group memberships

Metrics:

- ns/access check on host reference platform
- CPU cycles
- peak/steady memory
- descriptor-cache hit ratio
- allocations per check (target zero on hot path)

## 9. Namespace operation security semantics

Explicitly verify:

```text
rename/move same FS -> security preserved
copy -> destination inheritance by default
CloneFile -> destination inheritance by default
explicit preserve-security copy/clone -> authorized only
```

No operation may accidentally inherit security merely because data extents are shared.

## 10. Raw-media threat test

Demonstrate and document that ACLs alone do not encrypt plaintext data.

When an encrypted security-domain prototype exists, test raw image inspection with keys absent/present separately from ACL enforcement tests.

## 11. Failure injection

Inject failures while:

- creating a new descriptor
- switching an object to a new descriptor
- updating owner
- changing inheritable ACL
- deleting an unused descriptor
- updating security-domain policy

After every simulated crash, objects must reference either the old valid descriptor or the new valid descriptor, never partial security data.

## 12. Benchmark reporting

Every security optimization must report at least:

- CPU
- peak RAM
- descriptor metadata bytes
- block reads/writes
- write amplification
- access-check latency p50/p95/p99

A faster ACL mechanism that materially weakens portability or enforcement fidelity is not an acceptable optimization.

## 13. Trusted snapshot backup authority

[ADR-075](../adr/ADR-075-revocable-backup-capability.md) requires a host-granted,
filesystem-scoped backup capability. A filesystem-neutral harness must cover
missing/wrong-scope authority, revocation between operations on existing and
duplicated handles, denial before data disclosure or mutation, and cleanup after
revocation. Verify consistent reads after live permission changes and deletion.

Qualify concurrent authorization/revocation admission in each host adapter and
keep feature discovery separate from caller privilege. Test explicit re-grant
without reviving old grants. Ordinary-user historical access and rich ACL
mapping remain separate gates; this capability does not establish their policy.


Run `cargo test -p afsplus-vfs --all-features` for the backup service harness.
Require all denied operations to leave backend call counters and caller buffers
unchanged. Check reader-budget exhaustion, duplicate leases, busy deletion,
cleanup after revocation and new grants without old-reader resurrection.
An in-backend probe must observe revocation excluded throughout the read, and
a concurrent read/revoke trace must complete the admitted call before admitting
no further reads under that grant. The consumer facade must not expose the
unchecked backend hook.

Use one consumer for an independent provider and AFS+. Compare original names,
metadata and streamed bytes after live content changes, deletion and remount;
old-service grants/readers must fail on the remounted service. Both providers
change live protection metadata while historical reads retain the captured value.
Rich ACL transport and actual OS permission evaluation/authentication require
their own integration tests; raw protection preservation does not qualify those
bridges. Separate destination authority follows
[ADR-077](../adr/ADR-077-separate-restore-authority.md).


Plant a valid-CRC captured object-map leaf that omits a directory's child.
A direct absent-object lookup can return not-found, but directory enumeration
must report corruption because it follows an existing authoritative reference.
The inspecting mount and reader must issue zero writes/flushes; the exhaustive
checker must independently reject the damaged image.


## 14. Destination restore authority

Run `cargo test -p afsplus-vfs --all-features` for
[restore integration tests](../crates/afsplus-vfs/tests/restore.rs), compile-fail
role/facade checks and in-backend admission probes. Follow the
[restore contract](../docs/13-filesystem-api-v2.md#9-destination-scoped-restore-extension).
Require every denied operation to leave provider counters and caller buffers
unchanged. Cover foreign grants/handles, both grants on hard links, duplicated
handles, fresh grants without handle resurrection, cleanup after revocation,
and budget reservation before creation. Rejected names must have no backend
effects; provider errors must release reserved capacity.

An in-backend probe must observe every applicable permit held during writes
and links. A concurrent admitted-write/revoke trace must drain the write and
deny subsequent writes. Compile-fail cases must reject cross-role grants and
consumer access to privileged backend hooks.

Run one restoration job on an independent provider and AFS+: nested names,
streamed bytes, zero gaps, a hard-link alias, protection and all three timestamps.
On AFS+, remount and compare exact restored values, sparse allocation and link
identity. Verify an outside sentinel and an older snapshot independently;
require the exhaustive checker to report no errors or warnings. Reject a
nonempty destination, unrepresentable protection and invalid timestamps;
metadata refusal must issue zero device writes and flushes.

These tests exercise a host library with memory devices. Native authentication,
namespace races, IPC cleanup/cancellation, archive integrity, complete metadata
transport and crash-safe archive completion need their own provider/consumer
gates under Q5 and Q11.


## 15. Captured allocation enumeration

Run `cargo test -p afsplus-core snapshot_allocation_pages --all-features` and
`cargo test -p afsplus-core ordinal_page_checks --all-features` for captured
allocation and cross-page corruption checks. Enumerate a fragmented extent tree
with page sizes 1, 7 and 64, written and unwritten records, a 1 TiB gap,
reservations beyond EOF, direct files, rounded tails and empty files. Preserve
a final rounded allocation ending at 2^64 through actual preallocation, snapshot
creation and remount without classifying its representable offset/length as corruption. After live
truncation and remount, require exact captured coverage, bounded device reads,
zero writes/flushes and stale-handle refusal. Reject zero/excessive limits,
out-of-range ordinals and directory objects. An independently encoded valid-CRC
overlapping map must fail when the overlap straddles a page boundary.

The VFS backup harness must reconstruct the same bytes through allocation
coverage and ordinary reads on both independent and AFS+ providers. Clip rounded
ranges to logical EOF. Invalid limits and revoked grants must not call providers;
an in-backend probe must observe authority held throughout enumeration.
These checks qualify source enumeration. Destination reservations and the two
[archive modes](../adr/ADR-078-backup-preservation-modes.md) require their own
restoration, unsupported-provider and completion oracles.


## 16. Destination reservation restoration

Run `cargo test -p afsplus-vfs --all-features` for checked reservation admission,
unsupported-provider refusal and exact AFS+ restore fixtures. Require explicit
host byte limits, zero/overflow/excessive-request refusal before provider calls,
revocation, in-backend exclusion, unchanged logical size and written bytes.
The same restore job on independent and AFS+ providers must reserve unwritten
coverage within a hole and beyond EOF. AFS+ remount must preserve exact coverage,
including the final rounded 64-bit block; unsupported alignment must issue zero
device writes and flushes. Require a clean exhaustive checker verdict.

Run `cargo test -p afsplus-core snapshot_preallocation_publication --all-features -- --nocapture`
for every modeled preallocation publication cut with a retained snapshot.
Each recovered record must equal the complete old or new record, with exact
written bytes and unchanged captured allocation. Check both selectable slots.
Report old/new outcome counts. This is a host crash model, with its ordinary
subset/torn-write limits; actual devices require separate qualification.

Qualify later writes into reservations under near-full COW and snapshot pressure,
plus bounded traversal of fragmented existing layouts, before claiming capacity
reservation guarantees for constrained profiles. The per-operation byte limit
alone cannot prove either property.

## 17. Captured metadata inventory knowledge

Run `cargo test -p afsplus-vfs --all-features` for the default-provider and
independent-provider inventory oracles. Missing enumeration support must validate
the captured object and report both inventories uninspected. Missing objects
must preserve their error category. A provider with inventory support must use
captured values despite live changes. Wrong-service and revoked readers must
fail before provider calls; a fresh grant cannot reactivate an old reader. The
in-backend probe must observe the admission lock held during the fallback's stat.
The AFS+ remount fixture must report uninspected inventories and issue zero
writes/flushes, with a clean checker result. These gates qualify knowledge and
authority, not lossless inventory transport or actual host authentication.

## 18. Opaque captured metadata transport

Run `cargo test -p afsplus-vfs --all-features`. Enumerate independent-provider
attribute and security channels one entry per page, preserve exact keys/encoding
identifiers and stream unknown binary values through two-byte buffers. Include
empty values and embedded zero/non-UTF-8 bytes; change live inventories after
capture and require exact captured results. Descriptor size must equal collected
bytes. Invalid page sizes, cursor/key strings and overflowing read ranges must
fail before provider calls. Reject duplicate/nonprogressing keys, nonterminal
empty pages, excessive descriptors and invalid provider read counts.

Verify held operation permits inside provider calls. Revocation must block
further enumeration/reads with no provider calls or output-buffer changes.
The AFS+ remount fixture must explicitly return `NotSupported` for absent
transport methods and perform zero writes/flushes. Inventory knowledge does not
replace transport. Lossless destination installation and actual native provider
support require independent gates.

## 19. Staged opaque destination metadata

Run `cargo test -p afsplus-vfs --all-features` for the
[staged restore contract](../adr/ADR-083-staged-opaque-metadata-restore.md).
The independent provider oracle installs exact unknown binary values and empty
values while keeping active metadata absent until finish. Closing the caller's
object handle must not release the lease retained by an upload. Abort, premature
finish, revocation and write failure must return staging and handle budgets.
Oversized or invalid descriptors and excess chunks must fail before backend
calls. Foreign services and revoked grants must not write or publish. An
in-backend probe verifies permits during begin, write and finish.

Inject a partial staging-write error, an error before publication and an error
after complete publication. Require permanent failure of the damaged upload,
no success result and exactly the allowed old/complete-new active value.
The remounted AFS+ refusal fixture must issue zero writes/flushes. These are host
memory-provider oracles; durable staging requires crash tests, cleanup recovery
and measured RAM/I/O qualification before advertising native support.


## 20. Destination allocation readback

Run `cargo test -p afsplus-vfs --all-features` plus
`cargo test -p afsplus-core allocation_readback` and
`cargo test -p afsplus-core live_allocation_pages` for
[ADR-089](../adr/ADR-089-restore-allocation-readback.md).

The service oracle must reject invalid limits before provider invocation,
refuse unsupported enumeration, and validate page progress, bounds and ordering.
Include nonterminal empty pages, excessive entries, wrong cursors, zero-length
and overlapping ranges, and an end exceeding 2^64. A range ending exactly at
2^64 must succeed. Foreign services and revoked grants must fail without provider
calls; the in-backend probe must observe the operation permit held.

The AFS+ fixture compares one-entry pages for written data, holes, unwritten
reservations beyond EOF and the final rounded address block, then synchronizes
and remounts to compare the same allocation and logical size. Core queries must
issue no writes or barriers. Compare empty/direct/tree layouts and show that
live mutation changes committed readback without changing a retained snapshot's
layout. Invalid cursors/limits and directory queries must preserve the no-write
contract. An open mutation window must cause explicit refusal without a flush;
after explicit window commit, queries must show its committed allocation.

These gates qualify scoped readback, not the complete preservation consumer.
The consumer must bind archive allocation to sparse data, restore reservations,
compare byte coverage independently of extent segmentation, and refuse
unsupported or mismatched destinations. Native ownership, concurrency and
resource evidence retain separate gates.
