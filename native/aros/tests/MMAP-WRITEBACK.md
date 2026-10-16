# Atomic range writeback primitive (host only)

`afsplus_aros_writeback_atomic` writes a positioned, non-extending range in one
COW transaction even when the global or inode policy selects private in-place
updates. It leaves both policies and the open handle position unchanged. The
caller must keep the source buffer stable until return and freeze every writable
mapped alias before computing/checksumming it. This primitive does not provide
that coherence protocol, register mappings, invalidate aliases, or advertise
`FSV2_CAP_PAGING`. FFI revision 23 appends it without changing prior signatures.

Actual core, VFS and FFI tests passed: all three global/inode policy combinations,
durable content after remount, read-only snapshot refusal, and non-extending
range validation (existing VFS Invalid maps to DOS 210; count remains unchanged).
230 injected crash states expose the whole old or new three-page range; ordinary
in-place writes form a negative control with 28 torn/error states. Removing the
forced-COW condition is a compiled mutant rejected at runtime. The two existing
snapshot tests passed in the same run. API coverage and clippy of the four changed
libraries with `-D warnings` passed. Existing stable-toolchain dependency warnings
are retained in raw logs.

Reproduce with `cargo test -p afsplus-aros-ffi --test mmap_writeback --test
mmap_snapshot -- --nocapture` using an isolated target directory. Evidence and
hashes: `mmap-validation/afs-writeback-proof-001/result.json` in the workspace.
The earlier failed attempts are retained. There is no guest or SMP qualification.
