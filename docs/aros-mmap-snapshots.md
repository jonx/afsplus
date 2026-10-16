# Native private-mapping snapshot backing

The additive C boundary revision 22 exposes `afsplus_aros_clone_open_snapshot`
(API_V2 group). The source is an existing readable file handle; the result is
an immutable, independently positioned file handle closed normally. Public
object-ID lookup continues to hide orphan objects. Size, seek, examination
and comments use the live handle so they survive unlink.

On a writable shared-extents volume with OPEN_UNLINKED, the core clones into
the existing reserved orphan directory. It uses the ordinary CloneFile CoW
reference accounting and the existing orphan-name/crash-cleanup encoding.
It copies no file data. Closing the last handle runs ordinary bounded orphan
cleanup. No new disk format or capability bit is introduced.

`api/filesystem_paging_v2.h` defines a filesystem-neutral, fixed 48-byte native
packet, independent of the full shared paging capability. The handler takes
an open DOS cookie and fills a caller-allocated FileHandle. Failure leaves
the output unpublished. Allocation of the native cookie precedes cloning;
any later setup failure closes the Rust clone before releasing the cookie.
The source position is never changed. Successful output reports REFLINK or
RAM backing; it never aliases the mutable source's open description.

A caller may explicitly allow the copy fallback. A read-only mount or missing
clone capability then materializes immutable 64 KiB chunks inside a single
handler dispatch. Short reads are completed; premature EOF or any allocation
or I/O error discards the unpublished cookie and every chunk. Separate clone
handles can share these immutable chunks with independent positions. The
last close, or packet-context destruction, releases all of them. Write and
truncate requests are refused. This fallback is separate from the normal
no-data-copy fast path and from lazy physical page population in the kernel.

The handler is single-task. Commit timers and other DOS packets are processed
outside the active snapshot packet; trackdisk/NVMe completion callbacks
transfer blocks and do not call the mutable mounted Volume. The existing
boot-lifetime medium claim forwards a second handler with the same exact
key. It does not establish exclusion against raw writers, device aliases or
other overlapping medium keys. Exclusive ownership of the mounted volume is
therefore still required; multi-mount coherence is not claimed.

`FSV2_CAP_PAGING` remains unadvertised. Shared mmap coherence, page invalidation,
writeback arbitration, partial munmap and POSIX SIGBUS delivery are separate
unfinished contracts. These snapshot tests do not qualify any guest mapping,
scheduler, hardware or SMP behavior.

## Verification

`tools/test-mmap-snapshot.sh NEW-OUTPUT` accepts `AROS_SOURCE` and
`AFSPLUS_AROS_SDK_ROOT` (the latter points to `bin/apple-aarch64`) and saves
individual build/test logs. It runs only the named snapshot tests and packet
matrices; its C compilation uses generated SDK configuration and source DOS
headers on the host.

* Core: source data spans three blocks; clone writes zero data blocks and
  establishes three shared references. Source rewrite and unlink preserve
  the snapshot, including after remount. The independent filesystem checker
  accepts the image and the existing orphan cleanup.
* Real C FFI: snapshot size/read/seek survive source mutation and unlink;
  writes are refused and unsupported capability leaves the output untouched.
* ASan/UBSan packet matrix: clone dispatch, both explicit fallback classes,
  short reads, immutable bytes, seek/EOF, independently positioned shared
  backing, each of four allocation failures and non-fallback I/O errors.
* Complete existing packet regression under ASan/UBSan. A stale block-scope
  buffer in its trace-counter fixture was reset before the next packet.
* Native C packet layer linked with the actual Rust static library: real
  writable CoW clone after source rewrite/unlink, and a real read-only
  remount using RAM fallback, including examination and cleanup.

The AArch64 packet object and DOS mapping consumer compile/link separately.
Actual guest validation must record its own result; host PASS is insufficient.
