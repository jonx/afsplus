# Milestones and Acceptance Gates

Active objective: **AFS+ Mountable Alpha-0** — the same image must support
create/read/write/truncate/rename/fsync, crash replay, and a clean checker
through a host mount and a MacAROS adapter. Format epoch 1 remains unfrozen.

| ID | Milestone | Status | Exit criteria |
|---|---|---|---|
| M00 | Reader format frozen | Not started | Reference image can be independently decoded |
| M01 | Portable reader | Partial | macOS/Linux/AROS builds, fuzz clean |
| M02 | Formatter | Prototype complete | reader/formatter round-trip |
| M03 | RW core | Prototype complete | create/read/write/rename/unlink on images |
| M04 | Journal | Prototype complete, format experimental | exhaustive crash-point suite passes |
| M05 | Checker | Prototype complete | corruption corpus detected safely |
| M06 | AROS handler | Partial: Hosted S0, crash replay and S1a core SYS pivot qualified; S1b session, in-tree build and later platforms pending | classic apps operate without recompilation |
| M07 | FS API v2 | Portable subset implemented | 64-bit and capability tests pass |
| M08 | FUSE | Host mount and Hosted bidirectional same-image S0 qualified | same image read/write on host and AROS |
| M09 | Catalog | Not started | multi-million object enumeration fast path |
| M10 | Change stream | Not started | incremental index + rescan fallback |
| M11 | Grow resize | Not started | online/offline policy documented and tested |
| M12 | Classic reader | Not started | constrained profile implementation demonstrated |
| M13 | App qualification | Partial harness | Cargo/Git/Zed/Ferail/Moonstone workloads |
| M14 | Epoch 1 | Not started | format stability and external review |

M06/M08 first qualify AFS+ as a secondary same-image volume. The subsequent
system-volume ladder and AFS/FFS comparison contract are specified in
`testing/aros-system-volume-qualification.md`; a post-bootstrap `SYS:` pivot and
a boot-selected AFS+ volume are separate acceptance claims.
