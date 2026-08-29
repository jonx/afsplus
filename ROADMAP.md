# Roadmap

## Stage 0: Amiga-native design review

Status: initial review complete, continue subsystem-by-subsystem during implementation.

- review PFS3 source subsystem by subsystem
- document PFS3 atomic commit
- evaluate PFS4 B+ tree, tiny-file, and fragmentation ideas
- produce adopt/adapt/reject matrix
- revise AFS+ transaction and small-file ADRs before format freeze

## Stage A: make the specification executable

- establish Rust workspace and `afsplus-core`
- keep core disk semantics independent from host namespaces
- define language-neutral C ABI boundary
- define portable C implementation profiles
- freeze minimal reader subset
- implement tiny portable C reader
- create binary encoder/decoder tests
- create conformance images
- create cross-implementation Rust/C interoperability tests
- create sparse raw image backend
- create memory block backend
- create SliceBackend for partition/disk-image viewports
- create OverlayBackend for cheap writable test branches
- structured flight recorder
- deterministic test mode
- named fault injection points
- power-cut simulation backend
- operation record/replay
- tiny-cache test matrix
- fuzzing and property tests
- benchmark harness with CPU/RAM/I/O/write-amplification accounting

Observability, virtual block backends, deterministic replay, fault injection, and repeatable resource benchmarks are required before the writable format is considered stable enough to develop aggressively.

## Stage B: make images mutable

- Rust formatter
- portable C read/write baseline
- allocation regions
- object mutation
- B+ tree directories
- extent mapping
- shared-extent/refcount prototype for reflinks
- CloneFile and CloneRange semantics
- canonical portable principal model
- shared immutable security-descriptor prototype
- ALLOW/DENY ACL evaluation and inheritance vectors
- classic protection-bit projection without destroying rich ACLs
- prototype checkpoint-COW transaction engine
- prototype redo-journal alternative
- compare transaction engines under identical crash/write-amplification tests
- deferred reclamation
- checker
- explain APIs
- semantic image diff
- read-only retained-checkpoint viewport for debugging/recovery
- continuously compare Rust and C CPU, RAM, I/O, and semantic results

Do not freeze the transaction format until checkpoint-COW and redo-journal prototypes have been compared experimentally.

The epoch-1 extent model must support shared extents even if every clone API is not production-complete at the first writable milestone.

The epoch-1 security model must preserve rich ACL metadata on hosts that expose only a simpler permission view.

## Stage C: integrate AROS

- handler
- DOS compatibility
- Filesystem API v2
- modern 64-bit API
- clone/reflink capability API
- modern path semantics
- notifications
- health reporting
- trace streaming / developer attachment
- structured management APIs
- Rust/C integration boundary
- generic file-backed virtual block device for mounting disk images
- native AROS benchmark runner using the same workload descriptions
- classic/single-user security adapter
- optional AROS multi-user principal/security service integration
- strict/preserve/compat security mount modes

## Stage D: make it portable and pleasant

- FUSE host mount
- third-party probe kit
- compatibility profiles
- classic reader
- portable C `classic-rw` qualification
- portable C `full-portable` qualification where feasible
- JSON/structured tooling schemas
- host-side inspect/check/repair workflow
- easy sparse-image create/mount/fork/replay workflow
- cross-OS interoperability test matrix
- POSIX ACL/principal mapping adapter
- Windows ACL/SID mapping adapter
- cross-OS security round-trip and fidelity reporting

## Stage E: modern accelerators

- global catalog
- persistent change stream
- fast enumeration API
- production reflink/block cloning
- benchmark tiny-file storage alternatives
- evaluate rebuildable reverse map
- evaluate recursive directory statistics
- evaluate optional data checksums
- evaluate CloneTree separately from file/range cloning
- prototype shared subtree security domains
- benchmark security-domain policy updates against recursively materialized ACL changes
- separate design review for optional encrypted security domains / key hierarchy

## Stage F: production

- grow resize
- minimum-size query
- shrink/relocation after safe mover exists
- targeted scrub
- online repair where justified
- performance qualification
- low-memory qualification
- CPU-efficiency qualification
- Rust-vs-C resource qualification
- security access-check performance qualification
- real SSD qualification
- exhaustive crash-point qualification
- independent format review
- epoch 1 freeze

## Epoch 1 freeze gates

Do not freeze the format until:

- normal crash recovery never requires a full-volume scan
- deterministic crash injection covers every transaction boundary
- NO_CHANGES performs zero media writes
- metadata ownership can be explained from supported tools
- shared extents cannot be freed while referenced by any live object or retained checkpoint
- sparse raw images and physical devices exercise the same disk format
- Rust and portable C implementations interoperate against the same conformance corpus
- performance reports include CPU, peak RAM, block I/O, flush count, and write amplification
- repair/check tools share format/invariant code where appropriate without making one implementation the specification
- machine-readable tools and errors are versioned
- classic/minimal reader profile is demonstrably implementable
- canonical ACL tests produce identical decisions in Rust and C
- unknown/unmapped principals survive round-trip without identity loss
- a host unable to enforce active security semantics cannot silently mount read-write in strict mode
- classic protection-bit edits do not accidentally erase richer security metadata
- Cargo/Git/Zed-style/Ferail workloads are qualified
