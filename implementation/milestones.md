# Milestones and Acceptance Gates

| ID | Milestone | Exit criteria |
|---|---|---|
| M00 | Reader format frozen | Reference image can be independently decoded |
| M01 | Portable reader | macOS/Linux/AROS builds, fuzz clean |
| M02 | Formatter | reader/formatter round-trip |
| M03 | RW core | create/read/write/rename/unlink on images |
| M04 | Journal | exhaustive crash-point suite passes |
| M05 | Checker | corruption corpus detected safely |
| M06 | AROS handler | classic apps operate without recompilation |
| M07 | FS API v2 | 64-bit and capability tests pass |
| M08 | FUSE | same image read/write on host and AROS |
| M09 | Catalog | multi-million object enumeration fast path |
| M10 | Change stream | incremental index + rescan fallback |
| M11 | Grow resize | online/offline policy documented and tested |
| M12 | Classic reader | constrained profile implementation demonstrated |
| M13 | App qualification | Cargo/Git/Zed/Ferail/Moonstone workloads |
| M14 | Epoch 1 | format stability and external review |
