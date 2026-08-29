# AFS+ Codex Handover

Status: active handover document for continuing architecture supervision and implementation review.

This file is intentionally not a replacement for the design conversation that produced AFS+. The full conversation contains useful rationale, discarded alternatives, workload examples, and the reasoning behind many constraints. When available, read the full conversation as historical context **in addition to this file**.

This document exists to tell you where the project stands now, which earlier ideas have been superseded, which decisions remain deliberately unresolved, and how to continue the role previously played by the architecture/review assistant.

---

## 1. Role you are inheriting

The user develops AFS+ with a coding agent. Your primary role is **tech lead, filesystem architecture reviewer, and development supervisor**.

Do not merely generate code. Continuously review whether implementation choices preserve the intended filesystem invariants, portability, resource discipline, crash consistency, and long-term format evolution.

The working loop is:

```text
user + coding agent implement a milestone
              |
              v
        tests / benchmarks
              |
              v
     you inspect code/diff/commit
              |
       +------+------+
       |             |
       v             v
 architecture OK   issue found
       |             |
       v             v
 next milestone   fix code/spec/ADR/test
```

A bug or implementation discovery is allowed to invalidate the specification. The specification is not sacred. However, a coding agent must never silently make an unresolved architecture decision simply because one implementation was easiest.

When implementation evidence contradicts an earlier design assumption, prefer:

```text
code -> measurement -> discussion -> explicit decision -> spec/ADR update
```

rather than forcing code to match an obsolete idea.

---

## 2. Context precedence

The full conversation is valuable and should be supplied to you when practical. It contains important intent and rationale.

However, the conversation also contains ideas that were later refined or rejected. Resolve conflicts using this order:

1. explicit current decisions and blockers in this handover
2. current repository code + tests that intentionally embody an agreed prototype contract
3. Accepted ADRs and current normative specification
4. current Proposed ADRs/docs, as proposals rather than commitments
5. historical conversation, for rationale and alternatives

Never treat an early conversational statement as a frozen format commitment if a later ADR, test, or handover note supersedes it.

When unsure whether something is historical or current, call it out instead of guessing.

---

## 3. What AFS+ is

AFS+ started from the AROS/Amiga ecosystem, but the format is deliberately portable and OS-neutral.

Its founding developer contract appears near the top of `README.md`:

> AFS+ is not being created to rebuild ext4, NTFS, or another conventional filesystem with Amiga branding. Its strongest contract is to expose directly to modern software the filesystem primitives that applications are otherwise forced to reconstruct, approximate, or continuously rediscover above the filesystem.

The intention is not to win a feature-checkbox contest. The intended differentiation is the combination of:

- low and bounded resource use
- modern 64-bit storage semantics
- strong and testable crash consistency
- portable independent implementations
- excellent developer observability and repairability
- efficient large-file and millions-of-small-file behavior
- semantic APIs for things applications repeatedly reconstruct themselves
- preservation of Amiga-friendly simplicity without making the disk format Amiga-only

The public API should remain filesystem-neutral. Other filesystem handlers may implement the same capability or report it unsupported.

---

## 4. Implementation language strategy

Rust is the primary modern implementation language because it improves development speed and removes several classes of memory-safety bugs that are particularly painful in filesystem code.

But **Rust is not the format**.

The project explicitly intends a portable C implementation/profile so AFS+ can be adopted by constrained and non-Rust systems.

Expected structure over time:

```text
                 normative AFS+ specification
                          |
              +-----------+-----------+
              |                       |
       Rust reference            portable C
              |                       |
      host tools / AROS          classic / AROS /
      FUSE / modern OSes         other OS ports
```

Rust and C must eventually cross-read/cross-write the same conformance corpus. Differences between independent implementations are useful because they expose ambiguous specification language.

Do not allow Rust implementation details to become undocumented format semantics.

---

## 5. Current executable implementation

Baseline implementation commit before this handover:

```text
74b14107466b4104470853cdcee3a8362acff8a9
Add first executable prototype: Rust workspace with checkpoint COW core
```

Always inspect current HEAD before working because newer commits may exist.

At that baseline, steps 1-5 of `implementation/peer-review-prototype-plan.md` are implemented and green:

- 29 tests
- zero clippy warnings
- ~3,592 lines
- `afsplus-format` builds as `no_std + alloc` on `aarch64-unknown-none`

### `crates/afsplus-format`

Prototype wire codecs with:

- explicit little-endian encoding
- no native-struct serialization
- CRC32C
- 32-byte common metadata header containing type, version, owner, generation, payload length, checksum
- bounds-first decoding
- normalization-preserving directory entry shape: original UTF-8 name plus comparison key
- binary key ordering
- current comparison-key encoder is identity only; pinned Unicode normalization/casefold tables come later

### `crates/afsplus-block`

Block provider plus:

- MemoryBackend
- sparse FileBackend
- trace/accounting backend
- deterministic fault backend
- power-cut recording/simulation backend

The current power-cut model enumerates all subsets of complete unflushed writes and representative torn-write states. Do **not** claim it models every physically possible torn/reordered state.

### `crates/afsplus-core`

Implemented:

- mkfs
- identification record
- alternating checkpoint A/B prototype
- bootstrap bump allocation via checkpoint high-water mark
- root object / root directory / object map
- first writable COW metadata transaction: create empty file
- shared reachable-state validator

The current writable create path writes four fresh metadata blocks, flushes, writes the alternate checkpoint, then flushes.

Bootstrap allocation intentionally never reuses blocks. This is scaffolding, not the final allocator.

### `crates/afsplus-check`

Verify-only checker with:

- shared validation code
- both checkpoint slots reported
- text output
- versioned JSON output

---

## 6. Important lesson already discovered by implementation

The first crash matrix exposed a subtle problem.

Because the bootstrap allocator never reuses blocks, an incorrectly ordered commit could sometimes be hidden by doing a full reachable-state validation at mount and falling back to the old checkpoint.

This is dangerous because it can make the crash harness appear to prove ordering correctness when mount is actually repairing around an invalid commit protocol.

Also, `validate_checkpoint_reachable()` currently walks every object/directory reachable from the checkpoint. Using that as the ordinary checkpoint-selection path would turn mounting a volume with millions of files into a full-volume scan, violating a founding requirement.

Therefore the design direction is:

```text
normal mount
    |
    +-- validate identification
    +-- inspect A/B checkpoint records
    +-- select newest structurally valid checkpoint using bounded root validation
    +-- mount without full-volume traversal

full reachable validation
    |
    +-- afsplus-check
    +-- shadow verification
    +-- test harness
    +-- explicit recovery/diagnostic mode
```

A deliberately bad commit order such as checkpoint-before-metadata-barrier must produce a crash state that fails the crash test. Normal mount must not silently perform an exhaustive scan and fall back in a way that masks that protocol bug.

Checkpoint CRC failure/torn checkpoint fallback remains legitimate.

---

## 7. Immediate hardening before allocator work

Before or alongside Stage 6, review/implement these points:

1. **Separate bounded checkpoint selection from full reachable validation.**
   - Normal mount must not scan all filesystem objects.
   - Full validation remains checker/test functionality.
   - Add a negative crash test using intentionally wrong checkpoint-before-metadata-flush ordering and prove the harness catches it.

2. **Harden short-buffer decoding.**
   - `Timespec::read()` must return an error on a buffer shorter than 12 bytes, never panic.
   - Audit public decode helpers for equivalent assumptions.

3. **Checked integer arithmetic.**
   - `generation + 1`
   - `next_free_block + N`
   - `next_object_id + 1`
   - future block/range arithmetic
   must return explicit errors on overflow, not panic/wrap.

4. **Equal checkpoint generations.**
   - Mount and checker must have one explicit policy.
   - Current direction: two distinct checkpoint slots claiming the same current generation are ambiguous/corrupt, not silently tie-broken.

5. **Power-cut model wording.**
   - Accurate claim: all full-write subsets plus representative torn writes for the modeled tail.
   - Do not claim exhaustive physical-device behavior beyond what is actually enumerated.

6. **Directory key invariant.**
   - With the current identity comparison-key encoder, enforce `entry.key == comparison_key(entry.name)`.
   - Later this must use the volume's pinned Unicode key rules.

7. **Unlinked dynamic objects.**
   - Until an explicit orphan/reclamation model exists, an object remaining in the authoritative object map with zero namespace references should fail validation.

These are hardening/correctness tasks, not opportunities to redesign unrelated subsystems.

---

## 8. The architecture blockers

These are deliberately unresolved. Do not silently settle them.

### Blocker 1: user-data COW policy

Open question:

```text
full data COW
vs
in-place overwrite when safe
vs
hybrid/per-file policy
```

Why it matters:

- old checkpoints
- committed content generations
- reflinks/shared extents
- race-free scanning of an exact content generation
- VM/database fragmentation
- write amplification

Reflinked ranges necessarily COW when modified. That does not by itself decide whether all ordinary file overwrites are COW.

### Blocker 2: fsync durability path

Checkpoint-COW is the leading transaction architecture, but a global checkpoint per tiny `fsync()` may be too expensive.

Do not treat "checkpoint COW vs redo journal" as a required binary bake-off anymore.

Current strategy:

1. implement checkpoint-COW correctly
2. measure fsync-heavy workloads
3. if needed, prototype a small durability/intent log layered with checkpoints

Do not build a second complete transaction engine unless evidence requires it.

### Blocker 3: allocation-state representation

This is the **next primary experiment**.

Problem: a COW authoritative bitmap can become self-referential because allocating a block for a new bitmap copy changes the bitmap itself.

Candidate first prototype:

- region-based allocation
- bitmap pages at deterministic/reserved physical locations outside normal allocation
- multiple generation slots per bitmap page, probably three, so current + fallback checkpoints can coexist while the next state is prepared
- authoritative allocation state must participate in checkpoint consistency

Do not implement full bitmap, delta-log, and spacemap engines just to compare them. Start with the simplest candidate that satisfies correctness and resource goals. Add alternatives only if measurement/correctness exposes a weakness.

### Blocker 4: retained content generations

Stable access to an exact older content generation is snapshot-like behavior.

Do not promise arbitrary historical generations unless the data blocks are actually retained.

Current contract should be conservative:

```text
OpenObjectGeneration(id, generation)
    -> exact generation if still retained
    -> GENERATION_NOT_AVAILABLE otherwise
```

Long-term retention policy remains open and interacts with data COW and reclamation.

### Blocker 5: rich security semantics

The project wants portable security metadata, but the initial NFSv4/Windows-like ACL design was identified as a high-risk area to freeze too early.

Current direction:

- reserve/version a shareable security-descriptor container
- preserve unknown/richer security metadata across simpler hosts
- do not freeze full cross-platform ACL evaluation semantics until real multi-OS adapters validate them

Never silently weaken stored security because the current host has a simpler model.

---

## 9. Stage 6: allocator experiment

After the immediate hardening above, proceed with the allocation experiment.

### First candidate

Prototype region allocation with bitmap pages whose storage location is predetermined/reserved, so bitmap metadata does not need to allocate itself.

Example reasoning for a 1 GiB region at 4 KiB blocks:

```text
1 GiB / 4 KiB = 262,144 blocks
bitmap = 262,144 bits = 32 KiB
```

Split large bitmaps into checksummed pages so a small allocation does not rewrite the entire 32 KiB bitmap.

Prototype multiple physical generation slots per bitmap page, likely three:

```text
slot A: allocation state generation G-1
slot B: allocation state generation G
slot C: prepare allocation state G+1
```

Exact layout remains experimental.

### Retired/quarantined storage is mandatory

Once reuse begins, free space needs at least the semantic states:

```text
ALLOCATED
RETIRED / QUARANTINED
FREE
```

A block removed from generation G+1 must not immediately become reusable if a still-selectable older checkpoint can reach it.

Conservative correctness beats aggressive reuse.

On uncertainty, quarantine/leak rather than reuse early.

### Mandatory reuse crash workload

Construct a sequence such as:

```text
G1: create A -> physical block X
G2: delete A -> X becomes retired, not free
G3: after safe checkpoint retirement, X may become reusable; create B reuses X
```

Inject power loss after every relevant write/flush during G2 and G3.

Allowed mounted states:

```text
G1: A exists and its bytes are correct
G2: A absent and X not unsafely reused
G3: B exists and its bytes are correct
```

Forbidden:

```text
A visible but X contains B's bytes
FREE block reachable from any selectable checkpoint
same non-shared physical block owned by two live objects
reuse before every checkpoint that can reach old contents is retired
```

This is a much more meaningful crash test than the bootstrap create-only matrix because reuse makes stale-but-valid block contents dangerous.

### Metrics from the start

Track at least:

- metadata bytes written per allocation/free
- bitmap pages written
- flush/barrier count
- blocks allocated
- blocks retired/quarantined
- blocks reclaimed
- reclaim latency
- allocator RAM footprint
- read/write amplification

AFS+ decisions should be driven by Pareto tradeoffs, not throughput alone.

---

## 10. Crash-consistency rules that must remain visible

A core lesson inherited from PFS3 and explicitly retained by AFS+:

> Never reuse a physical block while any retained/selectable checkpoint can still reach its previous contents.

The transaction model currently follows this high-level pattern:

```text
new user data as required
    -> durability ordering as required
COW authoritative metadata
    -> durability barrier
publish new checkpoint
    -> durability barrier
report durable commit
```

The exact data-write policy and future fsync log are unresolved, but checkpoint publication must never make references visible before the referenced state satisfies the promised durability contract.

Crash tests must verify allowed semantic states, not merely "mount succeeds".

A mount that hides transaction-protocol errors through expensive exhaustive recovery is not proof of correct ordering.

---

## 11. Feature evolution and removal

AFS+ deliberately tries to avoid making every experimental idea a permanent burden.

Features have lifecycle/state concepts such as:

```text
disabled
enabled but unused
active
experimental
deprecated
retired
```

Rules:

- once assigned, an on-disk feature identifier is never reused for another meaning
- new implementations may stop enabling an obsolete feature on new volumes
- an old volume with an active feature must remain readable according to its compatibility class or be explicitly converted
- removal of an active feature requires a conversion that eliminates its on-disk dependencies before the feature bit/state can be cleared
- unknown optional derived/discardable structures must never become correctness dependencies

This lets the project experiment without requiring every unsuccessful 2026 idea to remain enabled forever.

---

## 12. Important derived vs authoritative distinction

This distinction is foundational.

Examples of intended non-authoritative accelerators:

- global catalog
- reverse map, if kept derived
- directory statistics
- content fingerprint cache

The persistent change stream is different: it is **discardable but not reconstructible**. If history is lost, clients receive `RESCAN_REQUIRED`; current namespace state cannot recreate the exact event history.

Do not call every non-authoritative structure "rebuildable" if its history cannot actually be reconstructed.

The filesystem must remain correct without optional accelerators.

---

## 13. Developer-facing ideas and their maturity

Many API ideas are intentionally exploratory. Do not implement all of them just because they appear in the docs.

Promising/general primitives include:

- stable object IDs
- fast full-volume enumeration
- persistent change discovery
- reflink `CloneFile` / `CloneRange`
- bulk metadata operations
- explicit durability semantics
- explain/health interfaces
- access-intent/preallocation hints

More experimental items include:

- `AtomicBatch()` bounded namespace publication
- `SealContent()`
- security/content inspection feed
- security domains
- reverse map kept online
- recursive directory stats
- content fingerprint cache
- CloneTree

The project deliberately learned from examples such as Windows TxF: a plausible developer API is not enough. Real application usage and measured value should gate stabilization.

Catalog + change stream are considered among the strongest bets because analogous centralized enumeration/change-journal patterns have extensive real-world precedent.

---

## 14. Workload philosophy

Do not optimize AFS+ around only `dd` throughput.

Official/extreme workload classes include:

### Streaming / very large files

Measure:

- sequential throughput
- CPU per GiB
- fragmentation/extents
- metadata write amplification
- page-cache pollution
- preallocation behavior

### Git / source trees / millions of small files

Target 100k / 1M / 4M object scales.

Measure:

- create/stat/rename/delete
- metadata reads/writes
- directory traversal
- change-stream-based incremental discovery
- bulk metadata APIs
- cache footprint

### AI/LLM

Qualify:

- huge mostly immutable model files
- mmap/range access
- models larger than available RAM/page cache
- parallel page faults/readers
- dataset patterns ranging from huge shards to millions of tiny objects
- large checkpoint publication

Filesystem work must not claim to accelerate GPU compute. Its job is to remove storage-side waste.

### Security scanner / antivirus

Long-term design goal:

```text
scan once per exact content generation
```

A scanner should consume object IDs, content generations, persistent changes, and eventually stable exact-generation access where available rather than rescan every unchanged file.

This remains above the core allocator/transaction work and should not distract Stage 6.

---

## 15. Benchmark contract

Every important design comparison should report more than elapsed time.

At minimum where relevant:

- elapsed time / latency distribution
- CPU time/cycles
- peak RAM
- steady-state RAM/cache
- block reads
- block writes
- bytes written
- metadata bytes written
- flush/barrier count
- read amplification
- write amplification
- fragmentation/extent count
- recovery time

A change that is 20% faster but uses 4x RAM and writes 3x more data is not automatically an improvement.

A slightly slower design that cuts RAM and write amplification dramatically may be preferable, especially given AFS+'s low-resource goals.

---

## 16. Debuggability is an implementation requirement

Developer observability was intentionally designed early rather than bolted on years later.

Desired infrastructure includes:

- structured flight recorder
- transaction/checkpoint IDs on events
- deterministic replay
- named fault injection points
- power-cut simulation
- tiny-cache stress modes
- semantic image diff
- explain APIs
- machine-readable tools

When a serious bug is fixed, add a regression scenario reproducing it whenever reasonably possible.

A production AROS bug should ideally become a deterministic host-side image/trace reproducer.

---

## 17. Portability rules

The on-disk format must not embed AROS namespace syntax.

Volumes/Assigns and similar concepts belong in the AROS namespace adapter.

Names are stored as filesystem names, not `VOL:` paths.

Current direction for names:

- valid UTF-8
- original UTF-8 spelling preserved
- normalized/casefolded comparison key stored/derived for lookup
- binary tree ordering on a frozen key encoding
- Unicode table/version pinned by volume/format policy when real normalization arrives
- never locale-dependent collation inside the B+ tree

A tiny reader should not need a full locale collation implementation just to search a directory.

---

## 18. Security philosophy

AFS+ originated on essentially single-user systems but should not destroy richer rights when used elsewhere.

The important current invariant is:

> A simpler host must not silently erase or weaken security metadata it cannot represent.

Host administrator/root bypass is host policy, not a magic on-disk user.

Cryptographic confidentiality is separate from ACL enforcement.

Full portable ACL semantics remain an architecture blocker until real adapters validate them.

---

## 19. Things not to do right now

Do not respond to implementation uncertainty by adding large speculative specifications.

Do not implement all Proposed features.

Do not build a second complete transaction engine just because an old roadmap mentioned a bake-off.

Do not freeze full ACL semantics.

Do not claim arbitrary historical per-file snapshots while data retention is unresolved.

Do not optimize away safety around block reuse.

Do not let full-volume checker logic become ordinary mount logic.

Do not let AFS+/Rust-specific APIs leak into applications when a filesystem-neutral capability can express the same semantics.

Do not turn a benchmark result into a format decision without examining CPU, memory, write amplification, recovery, and low-space behavior.

---

## 20. Recommended files to read first

Read these before making architecture-sensitive changes:

```text
README.md
CODEX_HANDOVER.md
ROADMAP.md
implementation/peer-review-prototype-plan.md
crates/README.md

docs/03-on-disk-format.md
docs/05-directories-and-names.md
docs/06-files-and-extents.md
docs/07-allocation.md
docs/08-transactions-and-journal.md
docs/09-feature-framework.md
docs/23-pfs3-stage0-review.md
docs/26-debug-observability.md
docs/27-rust-implementation-strategy.md
docs/28-virtual-images-and-viewports.md

adr/ADR-020-checkpoint-commit.md
adr/ADR-021-deferred-reclamation.md
adr/ADR-022-cache-pinning.md
adr/ADR-023-developer-observability.md
adr/ADR-025-structured-management-api.md
```

Then inspect the current Rust crates and tests directly. Current code can reveal that a document has gone stale.

---

## 21. How to use the full conversation

If the user pastes or attaches the full prior conversation, read it.

The conversation is useful for:

- understanding why alternatives were rejected
- seeing the intended developer experience
- understanding the emphasis on low-resource operation
- seeing real target workloads such as Git, Ferail, antivirus, streaming, and LLMs
- understanding why Rust + portable C was chosen
- understanding why observability/fault injection are first-class
- recovering nuanced intent not worth duplicating into every normative document

Do not compress the conversation immediately into a new grand design. First compare its history against current repository state and this handover.

Useful pattern:

```text
conversation says X
current docs/code say Y

if X and Y agree:
    retain rationale
if X is older than Y:
    treat X as historical alternative
if conflict is unresolved:
    surface it explicitly
```

The detours are context. Preserve their lessons without reactivating superseded decisions by accident.

---

## 22. Expected interaction with the user

The user wants active supervision, not passive code generation.

When reviewing a milestone:

1. inspect the actual repository changes
2. state concrete findings, especially correctness issues, early
3. distinguish bugs from architecture choices
4. identify assumptions that accidentally decide blockers
5. ask for code changes only when needed; do not create process overhead for its own sake
6. update docs/ADR only when implementation evidence changes the design
7. keep the project moving toward executable tests rather than endless specification

Be willing to say that an earlier idea was wrong.

Be equally willing to defend an unusual design when the criticism is answered by measured workload needs and clean compatibility semantics.

---

## 23. Current next action

The immediate sequence is:

```text
A. harden current checkpoint/mount/crash prototype
B. add the negative bad-ordering crash test
C. begin Stage 6 region allocator prototype
D. introduce retired/quarantined storage
E. exercise real block reuse under exhaustive modeled crash points
F. measure allocator write amplification and RAM
G. only then decide whether reserved multi-slot bitmap pages are good enough or a delta/spacemap-style design is justified
```

The first major proof after the bootstrap prototype is not merely "allocation works".

It is:

> AFS+ can reuse storage after deletes without any selectable checkpoint ever observing stale metadata that points at newly reused content.

That invariant is the next major architecture gate.

---

## 24. Short resume instruction

If you need one sentence to resume the project:

> Continue as AFS+'s filesystem tech lead and architecture reviewer: inspect current HEAD, apply the hardening in `CODEX_HANDOVER.md` section 7, then supervise the Stage 6 allocator/reuse experiment without silently deciding the five architecture blockers.
