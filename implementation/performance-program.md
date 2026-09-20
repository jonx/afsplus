# Performance programme

Where the time of an AROS operation goes on 2026-09-19, measured, and the
order in which it is taken back. The target is Macaros Native on Apple
Silicon, where a device flush costs what a flush costs; Hosted, where it is
free, hides the largest item below. The reference is
[`tools/bench-hosted-aros.sh`](../tools/bench-hosted-aros.sh) at 1e45a23:
3.15 s against 1.43 s for the Fast File System (create 1.27 against 0.39,
list 0.08 against 0.06, read 0.22 against 0.33, rename 0.91 against 0.57,
delete 0.69 against 0.10 s), with 371,007 library calls, 8,056 device
flushes, 48,444 block writes and 1.33 million cache reads for 12,980
operations.

<!-- toc -->

- [What the measurements say](#what-the-measurements-say)
- [What each phase costs, after lot A](#what-each-phase-costs-after-lot-a)
- [The lots, in order](#the-lots-in-order)
- [How a lot is done](#how-a-lot-is-done)
- [Where it stands on 2026-09-19](#where-it-stands-on-2026-09-19)
- [Where the delete phase's flushes went, on 2026-09-20](#where-the-delete-phases-flushes-went-on-2026-09-20)
- [Memory](#memory)
  - [Where the allocations come from](#where-the-allocations-come-from)
  - [What lot J took back](#what-lot-j-took-back)
  - [What the table changed](#what-the-table-changed)
  - [What was left alone, and why](#what-was-left-alone-and-why)
- [Where it stands at the end of 2026-09-20](#where-it-stands-at-the-end-of-2026-09-20)

<!-- /toc -->

## What the measurements say

The host harness `crates/afsplus-aros-ffi/tests/zz_profile.rs` replays the
calls the packet layer makes for one DOS operation and runs under macOS
`sample`; the counters of `afsplus_aros_counters` give flushes, writes,
calls and cache reads per operation.

1. **Close is a flush.** `ACTION_END` on a written file calls
   `afsplus_aros_fsync` (`native/aros/afsplus_packet.c`), from the packet
   layer's first commit and before [ADR-121](../adr/ADR-121-delayed-group-commit.md).
   A write to a file the window created marks the window unloggable, so the
   fsync is a full checkpoint commit: 2 flushes and 12 block writes per
   created file against 0.01 and 2.1 without it, 504 µs against 201 µs per
   create on the host. ADR-121 names fsync, `ACTION_FLUSH`, inhibit,
   dismount, format, write protection and disk change as the durability
   points; a close is not one, and S3's contract already lets a cut lose a
   marker that was only closed. This is 60 % of the create phase on Hosted
   and, on Native, one device flush per file written.
2. **The commit walks its trees the slow way.** With the fsync gone, 65 % of
   a create is the window commit, amortised: `cow_tree::mutate_many` calls
   `children_from_node` on every internal node it passes for every operation
   of the batch, which clones every child key into a vector and checks the
   children for duplicates through a `BTreeSet`; a 512-operation commit does
   this some hundreds of thousands of times. Under it, `read_object` and the
   object-map lookup re-read and re-verify nodes the batch has already
   decoded, and the tree lookup for the directory does the same.
3. **The allocator scans from the start of a region.** `allocate_run_inner`
   walks the region's bitmap from bit 0 on every allocation, so filling a
   region is quadratic in its size: 13 % of a create.
4. **Three calls and three locks per path.** The packet layer resolves a
   path one component at a time, one `afsplus_aros_locate` and one lock per
   component, and `NameFromLock` is a `DupLock`, an `Examine` and a
   `ParentDir` per level; dos.library's `Rename` asks for both on both
   names, 28 packets. Ten library calls per create, 28 on average across the
   benchmark, 100 cache reads per operation.
5. **Orphans are cleaned one per transaction.** Each cleanup step is its own
   transaction with its own commit, 32 of them per idle tick, and a write
   that finds the volume below its room floor runs up to four of them
   before it proceeds. In a delete-then-create loop on a 32 MiB test disk
   that is 64 % of the time, and on Native every one of those commits is
   two flushes.
6. **Allocation.** A quarter of the host time is the allocator (`malloc`,
   `free` and the meter), of which 8 % is dropping the vectors
   `unicode_normalization` makes for a name key. On AROS every one of those
   is an exec `AllocMem` and `FreeMem` through `afsplus_bootlibc.c`, which
   is slower than the host's allocator by more than the host's share
   suggests.
7. **A created file is rewritten whole on every write.** A write to a file
   the window created reads its content back, changes it and writes it to a
   new run, up to 256 KiB. A program that writes a 200 KiB file in 8 KiB
   pieces moves 2.5 MB through the device for it. The benchmark writes each
   file once and does not see this; compilers and editors will.
8. **A directory is made outside the window.** `CreateDir` is an immediate
   transaction: it commits the open window and then itself, two commits
   and some four flushes for each of the 90 drawers the benchmark makes.
   ADR-121 point 8 foresaw it: operations join the window as the core
   learns to stage them.

## What each phase costs, after lot A

The benchmark lines now carry, per phase, the library calls, device
flushes, block writes and cache reads the phase took (7d6b1b7). At lot A,
2,560 files in 90 drawers on Hosted:

| Phase | ms | Calls per operation | Flushes | Writes per operation | Cache reads per operation |
|---|---|---|---|---|---|
| create | 365 | 12 | 750 | 3.2 | 53 |
| list | 115 | 3.4 | 2 | 0.07 | 25 |
| read | 200 | 12 | 0 | 0 | 47 |
| rename | 860 | 108 | 10 | 1.1 | 278 |
| delete | 730 | 8 | 2,170 | 4.3 | 99 |

Rename is calls: dos.library's 28 packets, each resolved and examined
through the boundary (lots C and H). Delete is commits: on a 64 MiB volume
the room floor is reached and orphans are cleaned one transaction each
(lot D). Create's flushes are the drawers (item 8) and its time the
commit (lot B).

## The lots, in order

| Lot | Change | Where | Gain expected | Proof |
|---|---|---|---|---|
| A | A close is not a durability point: `ACTION_END` closes, and only fsync, `ACTION_FLUSH` and the other ADR-121 points commit. ADR-121 records it. | `native/aros/afsplus_packet.c`, ADR-121, `docs/aros-native-bridge.md` | create 1.27 → about 0.5 s on Hosted; one flush per file gone on Native | packet matrix; harness: closes on a delayed mount cause no flush, and `SYNC` and an explicit fsync still do; the hosted DOS gate; S3 |
| B | The commit finds its way without materialising: `mutate_many` searches a node's items in place, checks a node's children once when it is first read, and keeps what it decoded for the batch. The allocator keeps a rover per region. | `crates/afsplus-core/src/cow_tree.rs`, `alloc.rs` | create commit 117 ms → a few ms per window; every phase | core tests of the trees and the allocator; the harness before and after; the image checker after every hosted run |
| C | One call per path: `afsplus_aros_locate_path` resolves a whole AmigaDOS path in the adapter, and `afsplus_aros_lock_name` returns the full name of a lock, so the packet layer makes one call where it made three to twelve. | `crates/afsplus-aros`, `afsplus-aros-ffi`, `api/afsplus_aros.h`, `afsplus_packet.c` | 28 calls per operation → about 10; rename most | packet matrix unchanged; FFI tests of the two calls, including `/`, `:` and the parent operations; hosted DOS gate |
| D | Orphans are cleaned in batches: one transaction cleans up to 32, an idle tick commits once, and the room floor takes one such step. | `crates/afsplus-vfs`, `afsplus-core` | delete 0.69 → nearer 0.3 s; 32 commits per idle tick → 1 | VFS tests of the count per transaction; harness delete-then-create; S3 |
| E | Names that are ASCII need no normalisation buffer; the key is made in place. | `crates/afsplus-format` | 5 to 8 % of every lookup | equality with the general path over the name corpus |
| F | A created file grows in place: a write at its end extends the run when the blocks after it are free, and the rewrite is the exception. | `crates/afsplus-core` | 200 KiB in 8 KiB writes: 2.5 MB → 200 KiB of device writes | harness writing in pieces, device write counter |
| G | The AROS build allocates from size-class pools over `AllocMem` slabs. | `crates/afsplus-aros-ffi/src/heap.rs`, `afsplus_bootlibc.c` | unknown on Hosted, measured there | heap tests; hosted bench |
| H | dos.library sends 28 packets for a `Rename`; a proposal for the Macaros fork (board task #11). Owner's decision. | AROS `rom/dos` | rename 0.91 → near FFS | the packet counts of the benchmark |
| I | A directory is made in the window, as a file is. | `crates/afsplus-core`, `afsplus-vfs` | 4 flushes per `CreateDir` → 0 | harness: 90 CreateDirs on a delayed mount flush at most once; the DOS gate; S3 |

A and B are independent of each other and of C; they are done first, in
that order, each with the hosted benchmark before it is called done. D
follows B, because a batched cleanup is one more batch through the same
trees, and I goes with it. E, F and G are taken by measurement after
that. H waits for the owner.

Lot A landed as the close that commits nothing: create 1.27 → 0.37 s on
Hosted, below the Fast File System's 0.45 s in the same boot; 3.15 → 2.36 s
in all, against 1.58 s; 8,056 → 2,956 flushes and 48,444 → 23,127 block
writes. It also found the STEADY probe of the DOS gate comparing two
readings of different states, which a commit on every close had made
identical by accident (`native/aros/tests/dos_compat_probe.c`).

## How a lot is done

A lot has its own worktree off `origin/main`, its own named tests and a
negative control, and it ends with the numbers of the harness and of the
hosted benchmark beside the numbers above. A lot that moves a bench phase
also runs `tools/check-hosted-aros-dos.sh`; one that changes what reaches
the disk or when also runs `tools/check-hosted-aros-s3.sh`. The full test
suite is not run.

## Where it stands on 2026-09-19

Lots A, B, C, D, F and I are on main, each with its named tests, a negative
control, the hosted DOS gate, the benchmark and, where it changes what
reaches the disk, S3. H went upstream as aros-development-team/AROS#1256 and
is applied to the local AROS tree. E was struck: `comparison_key` already
has an ASCII path, and the profile's share was drop glue instantiated in
that crate. The hosted benchmark reads 1.11 s against 1.15 s for the Fast
File System in the same boot (create 0.23 against 0.43, list 0.11 against
0.06, read 0.23 against 0.35, rename 0.23 against 0.21, delete 0.33 against
0.12 s), from 3.15 against 1.43 s.

| Phase | Calls per operation | Flushes | Writes per operation |
|---|---|---|---|
| create | 8 | 30 | 2.5 |
| rename | 6 | 10 | 1.1 |
| delete | 4 | 910 | 2.2 |

What is left, by measurement: the object-map batch encodes the same leaf once
per operation (lot B's report), and G, the pool allocator for the AROS build.

## Where the delete phase's flushes went, on 2026-09-20

Lot D2. The harness builds the benchmark's own tree on a 64 MiB disk, ten
trees of eight drawers of 32 files with the sizes of `afsplus_bench.c`, and
deletes it as the benchmark does, one `afsplus_aros_commit_due` after every
operation with the handler's 2 ms clock
(`crates/afsplus-aros-ffi/tests/zz_profile.rs`, ignored test
`delete_tree_phase`). Every flush is charged to the call that caused it, and
the checkpoints come from the flight recorder, so a line also says how many
transactions a call site published. The host reads 920 flushes where the
hosted benchmark reads 910: the same phase.

| Call site | Calls | Flushes before | Checkpoints before | Flushes after | Checkpoints after |
|---|---|---|---|---|---|
| delete file | 2,560 | 10 | 5 | 10 | 5 |
| remove drawer | 80 | 800 | 400 | 0 | 0 |
| remove tree | 10 | 100 | 50 | 0 | 0 |
| remove root | 1 | 10 | 5 | 0 | 0 |
| `commit_due` tick | 2,651 | 0 | 0 | 0 | 0 |
| whole phase | 2,651 | 920 | 460 | 10 | 5 |

The files were never the cost: 2,560 deletes are the 5 commits of the window
bound and nothing else. Every directory removal cost five checkpoints and ten
flushes: the open window committed first, then the removal's own transaction,
then three maintenance transactions behind it. Removing a directory now joins
the window, as making one did in lot I, and the phase costs the window
commits alone: 920 flushes and 6,568 block writes become 10 and 2,891.

The room floor was the other suspect and the table clears it for this phase:
the volume never comes near it, and neither the deletes nor the `commit_due`
ticks spend anything on cleanup. Where it is reached it was still
disproportionate, and a second harness test measures that on its own
(`room_floor_loop`: a volume filled to the floor, then a delete-and-create
loop that never goes idle). On a 64 MiB volume it costs 0.031 checkpoints per
operation with the old floor of an eighth of the volume and 0.019 with the
floor at what the next window can need. On a 16 MiB volume the same loop
costs 3 checkpoints per operation either way, because there it is not the
floor: it is the 1,024 blocks of headroom an operation keeps free before it
writes, on a volume with 542 blocks free.
What is left, by measurement: delete still commits 455 times for 2,650
operations on a 64 MiB volume, the room floor again; the object-map batch
encodes the same leaf once per operation (lot B's report); and G, the pool
allocator for the AROS build.

## Memory

What one mounted volume costs, on 2026-09-20, measured with the library's
own heap counters. The harness is
[`crates/afsplus-aros-ffi/tests/zz_memory.rs`](../crates/afsplus-aros-ffi/tests/zz_memory.rs),
a build of its own:

```text
cargo test -q -p afsplus-aros-ffi --features heap-profile --test zz_memory \
    -- --ignored --nocapture --test-threads=1
```

The `heap-profile` feature adds a count of the allocations the meter sees
and a histogram of their sizes; it is off by default because an atomic
increment per allocation would sit in the hottest path the library has. The
test disk is materialized, so the volume writing new blocks is not counted
as the library's memory, and every reading is the difference against the
heap before the mount.

**(a) The handler.** `tools/package-aros-dist.sh darwin-aarch64` builds a
4,648,696-byte `L/afsplus-handler`. It is a relocatable module, and its
relocations name its symbols, so it cannot be stripped: `--strip-debug`
takes it to 4,607,336 bytes, because it carries no debug information, and
the 1,992,792 bytes a full strip leaves would not load. Its sections are
1,631,776 of code, 348,904 of read-only data, 1,492,512 of relocations,
1,113,930 of symbol and string tables, and 1,843,228 of `.bss`, which costs
no file bytes and 1.8 MiB of memory at load. Both of those last figures are
answered below.

| Part | Bytes | Share of the code |
|---|---|---|
| `core::fmt` | 140,244 | 8.4 % |
| panicking and unwinding | 10,108 | 0.6 % |
| Unicode tables | 38,412 | 2.3 % |
| all of `core` | 462,544 | 27.8 % |
| all of `alloc` | 471,692 | 28.4 % |
| all of `std` | 187,888 | 11.3 % |

Formatting and panic machinery together are 9 % of the code and 3 % of the
file. `.bss` is another matter: 1,841,328 of its 1,843,228 bytes are one
symbol, `threads`, the fixed thread table of the AROS static pthread
library the module links. Nothing in AFS+ makes a thread.

**(b) A mount.** With the read cache off, a mounted volume holds 7,018
bytes. The cache is the rest, and it is taken in full when it is set:

| Buffers | Heap after the mount |
|---|---|
| 0 | 7,018 |
| 64 | 273,938 |
| 1024 | 4,273,298 |

**(c) A full delayed window**, 512 creates, 512 deletes and a 16 MiB write
on a 128 MiB volume with 64 buffers:

| After | Held | Peak |
|---|---|---|
| the mount | 5,386 | 12,928 |
| 512 creates | 289,956 | 1,714,103 |
| 512 deletes | 283,483 | 3,140,709 |
| a 16 MiB write | 408,211 | 3,140,709 |
| the commit | 382,299 | 3,140,709 |

The window holds little and peaks high: the commit of a full window of
deletes is 3.1 MiB, eleven times what the window held before it. The 16 MiB
write costs 125 KiB, because a write goes to the device as it is made.

**(d) Allocations per operation**, each phase measured between two commits
so that the phase before it is not charged to it:

| Operation | Allocations | Frees | Reallocations | Largest |
|---|---|---|---|---|
| create | 1,310 | 1,307 | 7.2 | 16,128 |
| lookup | 26 | 28 | 1.0 | 4,752 |
| rename | 1,711 | 1,707 | 9.8 | 11,328 |
| delete | 2,312 | 2,315 | 17.2 | 8,496 |

**(e) The sizes asked for.** Almost all of them are tiny: 710 of the 1,310
allocations of a create are 8 to 15 bytes and 220 are 4 to 7. Above them
stands one band that is neither small nor rare: 53 allocations of 4 KiB to
8 KiB per create, 167 per delete. Those are the block buffer
`cow_tree::read_node` makes for each node it decodes, one per tree node the
operation reads, against 59.6 cache reads per create. The largest single
allocations are that buffer and the vectors a commit builds.

What a 4 MiB 68000 machine could not live with: `Buffers=1024`, which asks
for more memory than the machine has; the 3.1 MiB peak of a full delayed
window, which is most of it; and the 1.8 MiB thread table the module
reserves before it serves a packet. What it can live with is the mount
itself, 7 KiB, and the 274 KiB of a 64-buffer cache. The first two are
answered below, and the third is gone.

### Where the allocations come from

Lot J. The meter can record a stack per allocation
(`heap_profile::trace` in `crates/afsplus-aros-ffi/src/heap.rs`, off unless a
measurement turns it on, because a stack walk costs far more than the
allocation it describes). The harness test `allocation_sites` in
`crates/afsplus-aros-ffi/tests/zz_memory.rs` turns it on for one phase at a
time and groups the stacks by their innermost AFS+ frame:

```text
cargo test -q -p afsplus-aros-ffi --features heap-profile --test zz_memory \
    -- --ignored --nocapture --test-threads=1 allocation_sites
```

It runs 128 operations, where the table of allocations per operation above
runs 512, so the counts below are of a shallower directory and are read as
shares rather than as the same absolute numbers.

Create, 829 allocations per operation:

| Site | Per create | Share |
|---|---|---|
| `TreeNode::decode`, under `cow_tree::read_node` for the commit's `upsert_node` | 267 | 32 % |
| `TreeNode::decode`, under `tree::lookup` for `object_map::lookup_lba` | 132 | 16 % |
| `TreeNode::decode`, under `tree::lookup` for `directory::lookup_entry` | 121 | 15 % |
| `TreeNode::decode`, under `directory::validate_root` at `load_mount_state` | 111 | 13 % |
| the read cache's own block box, under `CachedDevice` | 43 | 5 % |
| `TreeNode::decode`, under `tree::lookup` for `allocation_root::lookup_record` | 17 | 2 % |
| `to_vec` of a block, in the cache's `write_block` and in `upsert_node` | 15 | 2 % |
| `Box::new_uninit` for the decoded node, in `tree::decode_node` | 6 | 1 % |
| `vec![0u8; block_size]`, in `tree::lookup` | 7 | 1 % |
| `comparison_key`'s buffer | 10 | 1 % |

Rename, 1,299 per operation, and delete, 2,154, have the same shape:
`TreeNode::decode` is 87 % of a rename's allocations and 81 % of a delete's,
split between the descent (`tree::lookup`), the commit's tree walk
(`cow_tree::read_node` under `upsert_node` and `delete_node`) and, for a
delete, `directory::validate_root` at 552 per operation.

The expectation this table was made to test was that the small allocations
would be key and name clones spread over the descent, the window's overlay
and error paths. They are not spread at all. Every one of them is inside
`TreeNode::decode`, which turns a block into a `Vec<TreeItem>` whose every
item owns a `Vec<u8>` key and a `Vec<u8>` value: two heap allocations per
item, for every item of every node the operation touches, when a lookup
wants one item and a descent wants one separator. That is where the 710
allocations of 8 to 15 bytes and the 220 of 4 to 7 come from, and it is why
the sizes are those of a key and a child reference. The `format!` in the
error paths, the pending overlay's `BTreeMap` nodes and the flight recorder
together do not reach 1 % of an operation.

### What lot J took back

Four changes, in the order the table above put them, each measured on the
host with `zz_profile`'s `create_only` and the allocations per operation of
`zz_memory`. Every reading is against the lot's base, 5ecbcad: 30.9 us per
create, 1,310 allocations per create, 1,711 per rename, 2,312 per delete.

The times are the median of three runs, all five commits measured one after
another on the same quiet machine, because a host under other work reads two
microseconds either way.

| After | us per create | Create | Rename | Delete | Lookup |
|---|---|---|---|---|---|
| the base | 30.9 | 1,310 | 1,711 | 2,312 | 26.4 |
| the borrowed block buffer | 29.9 | 1,296 | 1,700 | 2,269 | 23.4 |
| the item that holds its own bytes | 29.4 | 186 | 221 | 632 | 15.2 |
| the descent path in an array | 28.5 | 173 | 211 | 594 | 12.2 |
| the image encoded once | 23.4 | 175 | 212 | 599 | 12.2 |

The time is the last change and the allocations are the third: cutting a
create from 1,310 allocations to 186 bought a microsecond and a half on the
host, where an allocation is a `malloc`. On AROS each of those 1,124
allocations is an exec `AllocMem` and a `FreeMem`, which is why the count is
worth holding to a bound whatever the host clock says.

**One block buffer, borrowed.** Every read of a tree node took a
`vec![0u8; block_size]` and dropped it. They come from a per-thread pool
now (`crates/afsplus-core/src/scratch.rs`); a borrow owns its buffer until
it is dropped, so a nested read is never handed the one its caller is
reading. It is not zeroed unless its size changed, because every caller
reads a block into it first, and the memset of a block was as much of the
cost as the allocation.

**A tree item holds its key and its value inside itself.**
`afsplus_format::small_bytes::SmallBytes` keeps up to 22 bytes without a
heap block, which is every key and every child reference the trees use, and
falls back to a vector above that. That is the whole of the 78 % above. It
is memory and not format: the same bytes reach the disk, and the C constants
test and the format roundtrip are unchanged. A tree item grows from 48 bytes
to 64. The descent carries its bounding keys the same way, and
`directory::validate_root` reads through the decoded-node cache instead of
decoding the root again per operation.

**A descent remembers its path in an array**, not in a `BTreeSet` made fresh
per lookup: a tree is at most sixteen nodes deep, and the set's first insert
took a heap block.

**A batch encodes an image once.** `stage_node` encoded a node every time an
operation passed through it: 1,500 encodes to write 62 images in a
512-operation batch. A staged image is deferred until its bytes are wanted,
to spill it, to read it back through the image, or to hand the writes to the
caller. The decoded cache is then load-bearing for a deferred block, so
nothing drops such a node without encoding it first. The memory bound is
that cache's own, which already follows the staged budget: at most eight
decoded nodes, about 64 KiB at a 4 KiB block, and none below sixteen staged
pages -- so the constrained profiles encode eagerly, exactly as before, and
their spill counts, spill reloads and staged residency are unchanged, since
a deferred image is in memory like an encoded one. A 512-upsert batch makes
62 encodes for 62 images with a decoded cache and 0 with none
(`a_batch_encodes_an_image_once_and_a_tiny_profile_encodes_eagerly`). This
is 18 % of a create on its own, 28.5 us against 23.4 measured back to back
against the commit before it, and it changes nothing about what a commit writes or when.

`crates/afsplus-aros-ffi/tests/heap_allocation_bounds.rs` holds the numbers
to these: 192 allocations per create, 234 per rename, 660 per delete and 14
per lookup, the readings above plus a tenth. Run against the base commit in
a separate worktree it fails on the first line, at 1,310 per create.

**What is left.** The largest remaining site is not below the 2 % the lot
aimed at: 30 allocations per create and 93 per delete, 20 % and 17 %, are
the `BTreeMap` nodes of the batch's own staged and decoded maps in
`cow_tree`, churned by the insert and remove of every staged image. Nothing
else reaches 7 %: the block copy in the cache's `write_block` at 6 %, the
`Vec` a lookup returns at 5 %, the vector `persist` builds at 5 %, and the
`Arc` of each decoded node at 4 %. A staged overlay that did not allocate
per entry is the next thing to measure, and it is a change to how a batch
holds its images, not another clone to remove.

### What the table changed

Four things, smallest first, each measured against the table above.

**The AROS heap carries no size header.** Rust's `GlobalAlloc` hands the
layout to `dealloc` and to `realloc`, so a block's size is known where it is
freed, and `crates/afsplus-aros-ffi/src/heap.rs` calls exec's `AllocMem` and
`FreeMem` with it through `afsplus_exec_alloc` and `afsplus_exec_free` in
`native/aros/afsplus_bootlibc.c`. The 16-byte header that file's `malloc`
puts before every block is gone from every Rust allocation; `malloc` keeps
it for the C shell, which frees without a size. With exec rounding to 16
bytes, an 8-byte allocation took 32 bytes and now takes 16, and a create
makes 710 allocations of 8 to 15 bytes. The host is unchanged in speed:
30.4, 31.7 and 31.8 us per create against 29.4, 30.9 and 31.7 before.

**The window's bound is a mount parameter.** The peak was the commit of a
full window, eleven times what the window held. It now comes from the same
memory profile the handler computes for `CACHE=AUTO`
(`auto_window_ops` in `native/aros/afsplus_handler.c`), through
`afsplus_aros_set_window_ops` (interface revision 19), and the peak follows
it:

| Changes the window holds | Peak |
|---|---|
| 512 | 3,140,709 |
| 128 | 1,088,324 |
| 64 | 883,244 |
| 32 | 879,836 |
| 16 | 883,564 |

Below 64 the peak stops falling, because the 16 MiB write is then the
largest thing in the run, so 64 is the floor. The handler takes 512 from
32 MiB of memory, 128 from 8 MiB and 64 below: a 4 MiB machine peaks at
about 880 KiB where it would have peaked at 3.1 MiB, and every machine the
qualified profiles run on keeps today's window.

**The handler costs a machine 1.8 MiB less before it mounts anything.** The
module's `.bss` was 1,843,228 bytes, of which 1,841,328 were one symbol,
`threads`: the fixed thread table of the AROS static pthread library the link
list named. It is taken at load, on every machine that has the handler in
`L:`, whether or not a volume is ever mounted, and on a 4 MiB 68k machine it
is most of the machine. AFS+ creates no thread.

What Rust's standard library asks of a thread library is eighteen entry
points -- a Mutex, a Condvar, spawn, join, detach, yield and sleep -- plus six
that compiler-rt's emulated TLS wants for a `#[thread_local]` static. That
second set is why dropping `-lpthread` from the profiles was not enough on
its own: `collect-aros` adds `-lpthread` by itself when a `pthread` symbol is
left undefined, and says so ("emulated-TLS dependency").
[`native/aros/afsplus_bootthread.c`](../native/aros/afsplus_bootthread.c)
answers all twenty-four, as `afsplus_bootlibc.c` and `afsplus_bootposix.c`
already answer libc and posix. The Mutex is real, over an exec
`SignalSemaphore`, because `std` takes it; the thread-local keys are one
task's table; a condition wait, a spawn, a join and a sleep on the packet
task each write a `bug()` line and fail, because a quiet success there is a
handler that waits forever with nothing written down.

| | Before | After |
|---|---|---|
| `.bss`, aarch64 | 1,843,228 | 596 |
| `.bss`, x86_64 | 1,584,873 | 577 |
| file, aarch64 | 4,728,872 | 4,549,256 |
| file, x86_64 | 6,617,824 | 5,875,248 |

The file also lost its local symbols. A relocatable module needs the symbols
its relocations name, and the AROS ELF loader reads only `sym->shindex` and
`sym->value` from an entry, the name reaching nothing but a debug line
(`rom/dos/internalloadseg_elf.c`); it loads `.symtab` and `.strtab` whole to
do it and frees them afterwards. `llvm-objcopy --discard-all` takes out the
locals and keeps every global: 732 symbols and 140,784 bytes on aarch64, 738
and 709,512 on x86_64, where a function per section makes the names longer,
off the package and off what the loader holds while it relocates.
That every relocation still resolves to the same section and offset was
checked entry by entry, 65,431 of them on aarch64 and 34,350 on x86_64.

Both ABI audits now refuse a module with more than 64 KiB of `.bss`, or any
single zero-filled object above 64 KiB, and name it. Run against the handler
built before this change they fail and name `threads`.

### What was left alone, and why

**A pool of the handler's own** (`CreatePool`, `AllocPooled`) was not made.
The measurement that would have chosen its puddle size says the blocks are
not there to pool: a mounted volume holds 413 live blocks after 512 creates
and 814 after a commit, and a dismount already returns every one of them,
because the meter reads zero held after an unmount. What a pool would buy is
fewer exec calls per operation, and that is a speed question for the hosted
benchmark, not a memory one; the header that was costing memory is gone
above.

**The buffers that start large** were looked for and not found where the
histogram pointed. The 4 KiB band is 53 allocations per create, and they are
the one block buffer `tree::lookup` takes per call, not a buffer that starts
large and stays empty: every one of them is filled by the block it reads.
Giving them a home would mean threading a scratch buffer through
`tree`, `directory` and `volume`, which is a wide change for no change in
what the handler holds -- the held and peak figures do not move when the
`cow_tree` read path is given such a buffer, which was measured: 1,310
allocations per create became 1,307.9. It belongs to a lot about allocation
count, not to this one.

**A smaller build for small machines** was not proposed. Formatting and
panic machinery are 9 % of the code and 3 % of the handler file, and an
`opt-level = "z"` build would take part of that: a speed trade for 3 % of a
file, worth making only with a measurement of what it costs the benchmark,
which belongs to a lot of its own.

**Garbage collection of unreferenced code** was measured and does not apply.
The x86_64 profile already gives every function its own section -- the module
carries 12,718 of them -- so `--gc-sections` looks like the obvious next
saving. It is not: `collect-aros` links a module with `ld -r`, and a
relocatable link has no entry point to keep anything alive from, so
`--gc-sections` collects nearly all of it. The link runs, and produces a
12,168-byte module: everything the handler is, collected. What drops
unreferenced `core` and `alloc` code here is the archive member, not the
section: a member nothing refers to is never pulled in, which is how the
thread table left.

## Where it stands at the end of 2026-09-20

The benchmark reads 0.93 s against 1.12 s for the Fast File System in the
same boot: create 0.21 against 0.40, list 0.10 against 0.08, read 0.20
against 0.34, rename 0.22 against 0.22, delete 0.21 against 0.09 s. It began
the day before at 3.15 against 1.43 s.

| Phase | Calls per operation | Flushes | Writes per operation |
|---|---|---|---|
| create | 8 | 30 | 2.5 |
| list | 3 | 2 | 0.01 |
| read | 8 | 0 | 0 |
| rename | 6 | 10 | 1.1 |
| delete | 4 | 10 | 1.1 |

Lots A, B, C, D, D2, F, G, I, J, K are on main and H is in AROS upstream. E
was struck. The flushes are what the window commits, everywhere.

What is left, by measurement: delete is the one phase still behind the Fast
File System, and its cost is no longer flushes; the staged and decoded maps
of a batch churn `BTreeMap` nodes, 20 % of a create's allocations and 17 %
of a delete's; the block cache copies a whole block into itself on every
insert; a lookup copies the value it found out of the node at the API
boundary. None of it has been measured on hardware: Hosted gives software
cost, and a device flush there is free.

