# 31. Extreme Workloads: Streaming, Git-Scale Trees, and AI/LLM

Status: workload architecture and qualification plan

## 1. Goal

AFS+ should not optimize only for ordinary desktop workloads. It should remain efficient across several intentionally extreme patterns that stress very different parts of a filesystem:

1. very large sequential files such as video, disk images, scientific data, backups, and model weights
2. millions of tiny files and metadata-heavy operations such as Git working trees, source trees, package caches, and build artifacts
3. AI/LLM workloads involving memory-mapped model weights, sharded model files, huge datasets, temporary artifacts, and large atomic checkpoints

The design rule is to expose generic primitives that help multiple workloads instead of adding format features named after one application.

## 2. Large streaming files

Typical patterns:

- multi-GB or multi-TB sequential reads
- long sequential writes
- one-pass consumption where cache pollution is undesirable
- media playback where latency stability matters more than metadata richness
- recording/capture workloads that append continuously

AFS+ should support:

- best-effort contiguous extent growth
- explicit preallocation/reservation
- large extent targets
- efficient sparse files
- asynchronous readahead hints through the host API
- sequential/random/one-shot access advice
- ability to drop already-consumed ranges from cache through the host VM/I/O layer
- Direct I/O capability where the host supports it
- low metadata write amplification while appending large files
- range-oriented statistics and fragmentation reporting

No compression policy should be forced onto media or already-compressed data.

## 3. Access-intent hints

Filesystem API v2 should expose advisory access intent similar in spirit to `posix_fadvise`, without changing correctness semantics.

Proposed hints:

```text
NORMAL
SEQUENTIAL
RANDOM
WILL_NEED
DONT_NEED
NO_REUSE
LATENCY_SENSITIVE
BULK_THROUGHPUT
MMAP_EXPECTED
DIRECT_IO_PREFERRED
TEMPORARY
IMMUTABLE_EXPECTED
```

Hints are per-handle or per-range where possible.

The filesystem/OS may ignore them.

They must never weaken durability or security promises.

## 4. Preallocation

Large-file users should be able to ask:

```text
Preallocate(file, offset, length, mode)
```

Modes may include:

- reserve logical space without changing file size
- ensure allocation for an existing logical range
- best-effort contiguous placement
- allow fallback to fragmented placement

Applications should not need filesystem-specific ioctls for ordinary allocation intent.

## 5. Git and millions of small files

Git performance is often dominated by metadata discovery rather than bulk throughput.

Git already uses filesystem monitoring and an untracked cache to avoid repeatedly scanning and `stat`-ing every file in large working trees.

AFS+ can make this particularly efficient because it already plans:

- persistent change stream
- stable object IDs
- fast global object enumeration
- B+ tree directories
- optional tiny-file optimization
- per-directory case policy
- efficient symlink semantics
- structured bulk metadata APIs

## 6. Native FSMonitor-style acceleration

A Git integration should be able to use:

```text
GetChangesSince(sequence)
```

instead of an external watcher where the host permits it.

The result can contain exactly the paths/object IDs whose working-tree-visible state changed since the previous Git command.

Benefits:

- no full tree scan
- no daemon required merely to remember transient events
- restart-safe monitoring
- direct recovery from application downtime

If the change-stream window has expired, Git receives `RESCAN_REQUIRED` and falls back safely.

## 7. Bulk metadata APIs

Millions of individual syscall/API round trips are expensive even when each lookup is fast.

Filesystem API v2 should prototype semantic batch operations such as:

```text
StatBatch(paths[])
StatObjectsBatch(object_ids[])
LookupBatch(directory_id, names[])
EnumerateTree(root_id, include_metadata=true)
```

These should return versioned structured records.

Batch operations must have bounded request/result sizes and permit streaming/chunking.

## 8. Directory-change generation

Directories should expose a generation that changes when their namespace contents change.

This can support efficient caches analogous to Git's untracked cache without requiring applications to infer correctness solely from coarse timestamps.

Example:

```text
dir object 42
namespace_generation = 918
```

If generation 918 is unchanged, a consumer can know that no child name was added or removed since its previous observation.

## 9. AI/LLM model-loading workload

Modern model runtimes commonly memory-map large model files so only needed pages are faulted into memory.

Typical files include:

- GGUF model weights
- Safetensors shards
- quantized model files
- tokenizer/config files
- adapters/LoRA files

AFS+ should therefore qualify:

- very large read-only files
- efficient mmap-backed random/sequential page faults
- parallel faults/readers
- range prefetch
- large aligned extents
- low fragmentation
- stable mappings across rename
- Direct I/O or pread-style range access when mmap is not desirable

The FUSE/host port must also be tested specifically for parallel mmap/page-fault behavior because user-space filesystems can have different mmap characteristics from native filesystems.

## 10. Sealed immutable content

Model weights, package objects, Git packfiles, downloaded artifacts, and many media assets become immutable after creation.

AFS+ should prototype an explicit sealed-content state.

Conceptually:

```text
SealContent(object)
```

After sealing:

- ordinary writes/truncates are refused
- content generation is stable
- content fingerprint may be computed/cached once
- antivirus verdicts can be reused against the generation
- reflink clones can reuse content identity
- allocator may treat the object as long-lived/read-mostly

Unsealing, if supported, is an explicit privileged mutation that creates a new content generation and invalidates derived content identity/security caches.

Sealing is generic and not LLM-specific.

## 11. Model cloning and local model experimentation

Large model variants and converted/quantized versions can consume huge amounts of storage.

AFS+ reflinks make cheap local experimentation possible:

```text
CloneFile(model.gguf, model-test.gguf)
```

The clone initially shares data extents. Subsequent range modifications use copy-on-write.

This is most useful when tools actually modify only a subset of the file. Full rewrites still write full new data and should not be misrepresented as cheap deltas.

## 12. Training/checkpoint workloads

Training or fine-tuning can produce very large checkpoint directories.

Useful primitives:

- preallocate large output shards
- sequential write hints
- `AtomicBatch` to publish a complete checkpoint set
- reflink/range clone for genuinely incremental formats
- persistent change stream for backup/indexing
- sealed checkpoint files after successful publication

Possible publication flow:

```text
write checkpoint shard A.tmp
write checkpoint shard B.tmp
fsync shards
BeginAtomicBatch()
Replace(A.tmp, A)
Replace(B.tmp, B)
Replace(manifest.tmp, manifest)
CommitAtomicBatch()
SealContent(A)
SealContent(B)
```

A crash exposes either the previous checkpoint set or the newly committed set, not an arbitrary mixture.

## 13. Dataset workloads

Datasets vary widely:

- a few enormous archive/shard files
- millions of images/documents
- append-only logs
- random small samples

AFS+ should therefore avoid one universal storage policy.

The same generic tools apply:

- sequential/random access hints
- global catalog
- batch stat/enumeration
- range prefetch
- tiny-file policy where measured beneficial
- content generations and fingerprints
- change stream

## 14. Page-cache pollution

A 200 GB video scan or model load should not necessarily evict every useful small-file metadata/cache page on the machine.

AFS+ host adapters should expose or map generic advice such as `NO_REUSE` and `DONT_NEED` to the host VM/cache layer.

The filesystem itself should keep metadata caching separate enough that streaming a huge file does not trivially evict essential metadata pages.

## 15. Large alignment and extent hints

Large immutable files can benefit from fewer, larger extents and alignment to storage/VM boundaries.

The allocator may accept advisory placement hints such as:

```text
preferred_extent_size = 2 MiB
alignment = 2 MiB
```

These are placement hints only.

The format must not require 2 MiB allocation units or hard-code a specific CPU huge-page size.

## 16. Direct I/O and zero-copy boundaries

AFS+ should support a clean mapping to host Direct I/O where available.

The filesystem should not invent GPU-direct semantics in epoch 1.

Instead it should expose:

- aligned range reads
- async/vector I/O capability where the OS supports it
- direct-I/O compatibility
- mmap-friendly backing

Future host platforms can build GPU/accelerator-specific pipelines above these generic contracts.

## 17. What AFS+ should not pretend to solve

AFS+ cannot make every LLM workload storage-bound or magically accelerate matrix multiplication.

Its role is to minimize storage-side waste:

- fast model load
- efficient page faults/range reads
- low cache pollution
- cheap clones
- fast metadata discovery
- robust checkpoint publication
- efficient datasets

Compute scheduling, GPU memory management, tensor placement, KV-cache layout, and model execution belong to higher layers.

## 18. Workload qualification principle

A feature should be accepted because it improves one or more generic workload classes under measured CPU/RAM/I/O cost.

Streaming, Git, and LLM workloads become official qualification suites so that future changes cannot make one class dramatically worse while optimizing another.
