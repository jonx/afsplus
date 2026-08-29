# 22. Lessons from PFS3 and the Proposed PFS4

PFS3 is a mandatory design reference for AFS+.

AFS+ should not be designed as if classic Amiga filesystem history consisted only of FFS. Professional File System 3 solved several practical Amiga filesystem problems extremely well, especially reliability, performance on modest hardware, fragmentation behavior, and useful native filesystem semantics.

## 1. Why PFS3 matters

PFS3 remains widely regarded as one of the strongest filesystems for classic 68k Amigas because it combines:

- good performance on very low-resource machines
- atomic metadata update behavior intended to keep a volume valid after crashes
- effective fragmentation prevention
- long filenames compared with classic FFS
- native Amiga permissions and comments
- mature tooling
- a filesystem-level deleted-file directory
- support on very old Amiga hardware

PFS3aio is still actively useful in the classic Amiga ecosystem and can run on 68000-class systems.

Its current limitations are equally instructive:

- legacy disk-size limits by modern standards
- legacy file-size limits
- no modern Unicode model
- no modern scalable global namespace index
- no modern checksums
- no modern change stream
- historical structures that accumulated as the format was extended

AFS+ should preserve the engineering discipline that made PFS3 attractive while avoiding the limits inherited from its original era.

## 2. Atomic commit is more relevant than copying a Unix journal model

PFS3's author described atomic commit as a defining reliability feature: after a crash or power failure, the disk should remain valid rather than requiring classic FFS-style validation.

AFS+ currently specifies a metadata redo journal.

Before format epoch 1 is frozen, the team must explicitly compare:

1. the proposed AFS+ journal design
2. PFS3 atomic-commit behavior and root-switch/update strategy
3. modern small-footprint transactional alternatives

The goal is not to use journaling because modern filesystems commonly use journaling. The goal is to obtain:

- atomic metadata updates
- bounded recovery time
- low write amplification
- low RAM requirements
- simple correctness proofs
- straightforward repair behavior

If a PFS3-like root-generation or shadow-metadata commit mechanism can meet those goals more simply than a conventional journal for some metadata classes, AFS+ should use the better mechanism.

The architecture therefore treats "transaction engine" as the requirement and "redo journal" as the current proposed implementation, not as an ideological requirement.

## 3. PFS4 is particularly relevant

Michiel Pelt described a planned PFS4 design that never became a released filesystem.

The publicly described goals included:

- B+ tree directory structure for scalability and large directories
- redesigned atomic commit
- automatic grouping of small files for space and performance
- built-in automatic defragmentation and improved fragmentation prevention

These ideas are unusually relevant to AFS+ because they map closely to independently identified requirements:

| Proposed PFS4 idea | AFS+ equivalent |
|---|---|
| B+ tree directories | B+ tree directory index |
| redesigned atomic commit | transaction engine / journal design review |
| grouping small files | optional inline/tiny-file storage |
| fragmentation prevention | extent allocator + locality |
| automatic defragmentation | future online extent relocation/maintenance |

The PFS4 design should therefore be investigated before AFS+ freezes these areas.

## 4. Tiny-file optimization should be elevated

The PFS4 proposal reinforces the importance of optimizing small files.

Modern developer workloads make this more important than it was on classic Amiga systems:

- Cargo registries
- Rust source trees
- Git metadata
- editor state
- configuration files
- package-manager metadata
- language-server caches

AFS+ already reserves an optional inline-data feature.

The implementation team should benchmark at least three approaches:

1. inline file payload in object records
2. packed small-object slabs
3. ordinary extents with allocator locality

Do not choose based on elegance alone.

Measure:

- disk overhead
- metadata writes
- read amplification
- RAM
- create/delete performance
- fsck/recovery complexity

## 5. Fragmentation prevention is preferable to mandatory defragmentation

PFS3 is known for behaving well with fragmentation compared with traditional FFS.

AFS+ should first make good allocation decisions:

- extend current extent
- allocate close to related metadata
- prefer sufficiently large free runs
- avoid scattering tiny writes unnecessarily

A future online relocation/defragmentation API may be useful, but good normal allocation behavior is more important.

## 6. Deleted-file recovery

PFS3's deleted-file directory is a useful user-facing feature.

AFS+ should not make a permanent filesystem recycle bin mandatory because:

- desktop environments may implement Trash policy differently
- servers and build volumes may not want it
- retaining deleted blocks affects free-space and privacy semantics

However, the object and transaction model should make an optional AROS Trash/recovery policy easy to implement.

Possible implementation:

```text
user deletes object
        |
desktop/DOS policy
        |
move/link object into hidden Trash namespace
        |
normal AFS+ transaction
```

This is preferable to embedding a compulsory recycle-bin policy into core allocation semantics.

## 7. Low-resource implementation is a core quality metric

PFS3 demonstrates that a robust and fast filesystem need not require modern amounts of RAM.

AFS+ must retain explicit memory budgets in benchmarks.

For each core operation record:

- minimum scratch memory
- typical cache memory
- maximum temporary allocation
- metadata pages pinned simultaneously

A feature that saves 5 percent CPU while adding hundreds of megabytes of mandatory metadata state is a poor fit for AFS+.

## 8. Study the source before freezing epoch 1

PFS3 is open source under a BSD license.

Before epoch 1, assign an explicit PFS3 study workstream covering:

- atomic commit/update logic
- anodes
- allocation
- directory representation
- update ordering
- recovery tooling
- cache behavior
- fragmentation handling
- deleted-file handling
- low-memory strategies

The result should be a written "adopt / adapt / reject" table.

AFS+ should not copy PFS3's obsolete limits, but it should not accidentally discard three decades of Amiga-specific filesystem engineering knowledge either.
