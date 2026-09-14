# Design Documentation Index

The design series explains how AFS+ is built and why. Numbers are permanent:
a document keeps its number for life and retired numbers are never reused.
[00-overview.md](00-overview.md) is the entry point. Decisions live in
[adr/](../adr/README.md); normative constants and invariants live in
[spec/](#spec); where the project stands lives in
[implementation/milestones.md](../implementation/milestones.md).

Every document in this directory opens with a navigation block linking the
ADRs, specification files, test plans and milestones it relates to; the block
format and the writing rules are in [DOCUMENTATION.md](DOCUMENTATION.md).

## Design series

| Document | Summary |
|---|---|
| [00 Overview](00-overview.md) | Scope, layered system architecture, what AFS+ is not |
| [01 Goals and Non-goals](01-goals-and-nongoals.md) | AROS concepts represented without sidecar files; explicit non-goals |
| [02 Architecture](02-architecture.md) | The four logical layers and their boundaries |
| [03 On-Disk Format](03-on-disk-format.md) | Encoding rules, identification, checkpoints, block headers |
| [04 Object Model](04-object-model.md) | Objects with stable numeric IDs, records and generations |
| [05 Directories and Names](05-directories-and-names.md) | B+ tree directories keyed by versioned comparison keys; UTF-8 spelling preservation; case policy |
| [06 Files and Extents](06-files-and-extents.md) | Extent mappings, sparse files, preallocation, shared extents |
| [07 Allocation](07-allocation.md) | Fixed-size regions, bitmap pages, reserved descriptor slots, retirement and quarantine |
| [08 Transactions, Checkpoints, and Recovery](08-transactions-and-journal.md) | Atomic metadata transactions, checkpoint COW, intent log, durability semantics |
| [09 Feature Framework](09-feature-framework.md) | Feature flags, lifecycle states, compatibility classes |
| [10 Global Catalog](10-global-catalog.md) | Optional rebuildable index for fast whole-volume enumeration |
| [11 Change Stream](11-change-stream.md) | Discardable, non-reconstructible change history and `RESCAN_REQUIRED` |
| [12 Metadata and Extended Attributes](12-metadata-and-xattrs.md) | Core record fields versus extended attributes |
| [13 Filesystem API v2](13-filesystem-api-v2.md) | Modern 64-bit operations without breaking the classic DOS ABI |
| [14 Paths and Namespaces](14-paths-and-namespaces.md) | Hierarchy inside the format, AROS volume/assign namespace outside it |
| [15 Rust, Zed, Git, and Modern Applications](15-rust-zed-modern-apps.md) | Workloads of a self-hosted AROS development environment |
| [16 Classic and Constrained Systems](16-classic-systems.md) | Bounded-resource profiles and what they omit |
| [17 Portability](17-portability.md) | `libafsplus` outside AROS, FUSE, host tools |
| [18 Third-Party Integration](18-third-party-integration.md) | Partition type GUID, probing, disk utilities |
| [19 Recovery and Maintenance](19-recovery-and-maintenance.md) | Checker, repair, resize and scrub principles |
| [20 Performance Architecture](20-performance.md) | Measured workloads, caching, amplification budgets |
| [21 Security and Corruption Handling](21-security-and-corruption.md) | Parser validation and corruption containment |
| [22 Lessons from PFS3 and the Proposed PFS4](22-pfs3-and-pfs4-lessons.md) | The mandatory Amiga-native design references |
| [23 Stage 0 PFS3/PFS4 Design Review](23-pfs3-stage0-review.md) | Subsystem review and adopt/adapt/reject matrix |
| [24 Filesystem Comparison Matrix](24-filesystem-comparison.md) | Architectural capabilities compared with other filesystems |
| [25 What People Actually Want From a Filesystem](25-filesystem-wishlist.md) | Wishlist and candidate differentiators; not requirements until an ADR promotes them |
| [26 Debugging, Observability, and Fault Injection](26-debug-observability.md) | Flight recorder, deterministic replay, fault points, explain APIs |
| [27 Rust Implementation Strategy](27-rust-implementation-strategy.md) | Crate layout, `no_std` core, C boundary |
| [28 Virtual Images, Overlays, and Filesystem Viewports](28-virtual-images-and-viewports.md) | Slice and overlay backends for images and partitions |
| [29 First-Class Content Inspection and Anti-Malware Support](29-first-class-content-inspection.md) | Scan-once-per-content-generation contract for security tools |
| [30 Portable Multi-User Security Model](30-portable-security-model.md) | Security descriptor container and canonical ACL candidate |
| [31 Extreme Workloads](31-extreme-workloads.md) | Streaming, Git-scale trees, AI/LLM access patterns |
| [32 Reflink and Clone Semantics](32-reflink-clone-semantics.md) | `CloneFile`/`CloneRange` and shared-extent rules |

| [33 Practical File System Design review](33-practical-filesystem-design-review.md) | Complete chapter review, corrected comparisons and executable/future qualification requirements |

## Platform integration documents

| Document | Summary |
|---|---|
| [Native AROS bridge](aros-native-bridge.md) | The C ABI boundary, native handler assembly and the AROS qualification gates |
| [AFS+ MacAROS Alpha-0 package](aros-alpha0-package.md) | Layout of the self-contained MacAROS Alpha-0 qualification package |
| [AFS+ Hosted MacAROS S1 image](aros-s1-image.md) | The manifested system image used by the post-bootstrap `SYS:` pivot |
| [macFUSE FSKit activation on macOS](macos-fskit-activation.md) | Enabling the macFUSE FSKit backend, including the diagnostic workaround |

## Rules for this directory

| Document | Summary |
|---|---|
| [DOCUMENTATION.md](DOCUMENTATION.md) | How documentation is written, where each kind of fact lives, and what the checker enforces |

## Spec

Normative, machine-oriented definitions. Changing any of them follows the
format-change procedure in [CONTRIBUTING.md](../CONTRIBUTING.md).

| File | Content |
|---|---|
| [afsplus_format.h](../spec/afsplus_format.h) | On-disk constants, magic values and record layouts as a C header |
| [disk-layout.md](../spec/disk-layout.md) | Logical volume layout; offsets marked TBD freeze at epoch 1 |
| [backup-envelope.md](../spec/backup-envelope.md) | Versioned PAX integrity/completion controls and namespace separation |
| [Backup object metadata](../spec/backup-object-metadata.md) | Exact preservation fields and explicit inventory knowledge |
| [Opaque backup values](../spec/backup-opaque-values.md) | Bound descriptor/data pairs for binary preservation metadata |
| [Verified archive scratch replay](../spec/backup-spool.md) | Chunk integrity, bounded replay and incremental publication |
| [Object inventory manifests](../spec/backup-inventory.md) | Complete counted descriptor groups and bounded preservation |
| [Sparse archive contents](../spec/backup-sparse.md) | Explicit GNU sparse admission, bounded maps and authorized content transport |
| [Archive allocation preservation](../spec/backup-allocation.md) | Bound allocation records, explicit recovery and semantic destination verification |
| [Bound regular-file groups](../spec/backup-file.md) | Exact metadata/content binding, complete opaque groups and explicit recovery losses |
| [Bound namespace groups](../spec/backup-namespace.md) | Directory inventories, hard-link identity and scoped restoration |
| [snapshot-records.md](../spec/snapshot-records.md) | Experimental persistent registry and lifetime leaf encoding |
| [invariants.md](../spec/invariants.md) | Core invariants a conforming implementation enforces |
| [compatibility-rules.md](../spec/compatibility-rules.md) | Mount decision algorithm and feature compatibility classes |
| [feature-registry.toml](../spec/feature-registry.toml) | Registered feature identities, classes and lifecycle states |
| [project-manifest.json](../spec/project-manifest.json) | Machine-readable project constants |

## API

Draft public and developer headers; changes follow the API-change procedure
in [CONTRIBUTING.md](../CONTRIBUTING.md).

| Header | Content |
|---|---|
| [filesystem_v2.h](../api/filesystem_v2.h) | Filesystem API v2 (design document [13](13-filesystem-api-v2.md)) |
| [libafsplus.h](../api/libafsplus.h) | Portable core library API |
| [libafsplus_reader.h](../api/libafsplus_reader.h) | Constrained reader profile API |
| [libafsplus_writer.h](../api/libafsplus_writer.h) | Bounded portable-C durable namespace writer API |
| [afsplus_aros.h](../api/afsplus_aros.h) | Versioned C boundary for native AROS handlers ([ADR-042](../adr/ADR-042-aros-c-boundary.md)) |
| [debug_observability.h](../api/debug_observability.h) | Flight recorder, fault injection and explain APIs (design document [26](26-debug-observability.md)) |
| [performance_hints.h](../api/performance_hints.h) | Access-intent hints and sealed content ([ADR-032](../adr/ADR-032-access-intent-hints.md), [ADR-033](../adr/ADR-033-sealed-content.md)) |
| [content_inspection.h](../api/content_inspection.h) | Content-inspection feed (design document [29](29-first-class-content-inspection.md)) |
| [security_acl.h](../api/security_acl.h) | Portable security metadata and ACL candidate (design document [30](30-portable-security-model.md)) |

## Profiles

Compatibility profiles answer which features a volume enables for a given
consumer ([09 Feature Framework](09-feature-framework.md)).

| Profile | Purpose |
|---|---|
| [reader-minimal.toml](../profiles/reader-minimal.toml) | Smallest read-only feature set |
| [boot-safe.toml](../profiles/boot-safe.toml) | Features a bootloader can depend on |
| [classic-rw.toml](../profiles/classic-rw.toml) | Read/write on constrained classic systems |
| [workstation.toml](../profiles/workstation.toml) | Full modern feature set |
| [full.toml](../profiles/full.toml) | Every standardized feature supported by the implementation |

## Examples

| Example | Shows |
|---|---|
| [classic-reader.md](../examples/classic-reader.md) | What a constrained reader implements and skips |
| [fast-enumeration.md](../examples/fast-enumeration.md) | Whole-volume enumeration through the filesystem-neutral API |
| [incremental-index.md](../examples/incremental-index.md) | An indexer consuming the change stream with rescan fallback |

## Proposals

Drafts awaiting team review; nothing there is adopted. Index:
[proposals/README.md](../proposals/README.md).

## References

External design references and provenance notes:
[references/REFERENCES.md](../references/REFERENCES.md).
