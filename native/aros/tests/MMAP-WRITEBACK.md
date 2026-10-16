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


The optional `FSV2_PAGING_WRITEBACK_FROZEN` native packet now reaches this
primitive through the serialized real handler and requires interface revision
23 and API-v2 support. Its fixed 64-byte request is distinct from the unchanged
48-byte snapshot request. Sync durability, integer overflow, 32-bit transfer
length, cookies and reserved fields are validated; output count changes only
on success. The caller's freeze/pin obligation remains mandatory and is not
implemented by this packet. No range registration or CAP_PAGING is provided.

`sh tools/test-mmap-writeback.sh NEW-OUTPUT` with AROS_SOURCE and
AFSPLUS_AROS_SDK_ROOT runs actual C-to-Rust durable/remount/position tests,
revision/group refusal, and the snapshot/ordinary packet regressions. Five
compiled handler mutants were rejected at runtime. Both aarch64 and i386
header ABI assertions passed. Packet attempt003 assumed DOS MODE_OLDFILE
was read-only; its failure is retained, and attempt004 follows the existing
FFS/SFS-compatible writable-oldfile semantics. No production access policy was
changed to satisfy that test. Evidence: writeback-packet-proof-001/result.json.
