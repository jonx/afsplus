# ADR-057: Qualify the Rust reference handler on plain M68000 emulation

Status: Accepted for A500-configured emulator functionality and recovery

## Context

ADR-056 qualified the complete native AROS handler only with an M68020-or-newer
CPU. Compiling the same source for M68000 first appeared to freeze during
mount and later during the first file creation. The AROS serial trace stopped,
but the emulator remained alive.

Visual inspection changed the diagnosis: AROS had opened a `Software Failure`
requester for `afsplus-handler`, reporting illegal instruction `0x80000004` in
`afsplus_format::bitmap::BitmapPage::decode`. The linked instruction at that
address was a `MULU.L` generated for `u32::checked_mul(32384)`. `MULU.L` is an
M68020 instruction and is invalid on the Amiga 500's M68000.

A separate earlier stop was caused by an external tail call lowered to a short
branch whose final link displacement did not fit. A temporary
`Vec::with_capacity(0)` standard-library change altered code layout enough to
avoid that symptom, but was not a correct or necessary fix once the backend
tail-call lowering was repaired.

## Decision

The plain-68000 reference profile uses
`native/aros/m68k-unknown-aros-m68000.json` and the toolchain deltas recorded in
`native/aros/toolchain/`:

1. M68000 external calls are not tail-call lowered until the backend can relax
   an out-of-range call to a representable absolute jump;
2. i32 multiply-with-overflow uses generic expansion on M68000/M68010;
3. target fusion of overflow arithmetic is disabled for those i32 multiplies
   before M68020;
4. long multiply instructions and their plain i32 selection pattern carry an
   explicit `AtLeastM68020` predicate; and
5. LLVM tests cover variable signed/unsigned multiplication and the exact
   constant-multiply-plus-select shape emitted by Rust.

The AFS+ gate compiles every C object and probe with `-m68000`, records the CPU
from the Rust target JSON, and disassembles the final handler before boot. Any
decoded `MULU.L`/`MULS.L` or corresponding `0x4c00..0x4c3f` first word rejects
the artifact. This audit is defense in depth; it does not replace the LLVM
tests or runtime gate.

The gate accepts an explicit installed LLVM directory through
`AFSPLUS_AROS_M68K_LLVM_LIB`. It exports `DYLD_LIBRARY_PATH` inside the script
because macOS System Integrity Protection strips `DYLD_*` while launching an
Apple-platform `/bin/sh`. The evidence binds the actual `libLLVM.dylib` hash.

The accepted A500-configured FS-UAE run uses:

- CPU target `M68000`, model `A500`, 8 MiB fast memory and no Zorro III memory;
- the matching official 2026-08-04 AROS boot floppy, system ISO and ROM pair;
- the full Alpha-0 create/read/write/truncate/rename/fsync/casefold matrix; and
- all six deterministic intent-log cuts, with four old and two new outcomes.

Every final image passed the strict host checker with no pending intent-log
record. Internal FS-UAE screenshots were inspected during the accepted run so
an AROS software-failure requester could not be mistaken for a serial timeout.

No `alloc`/`Vec` change is part of the accepted toolchain. No AROS or MacAROS
source change is required by this result.

## Consequences

- Stage 3 now has functional and crash-recovery evidence on both the
  M68020-or-newer reference profile and an A500-configured M68000 emulator.
- This does not establish the final classic memory budget, acceptable
  performance, physical-media behavior, or physical Amiga 500 compatibility.
- The Rust compiler/PAL remains experimental. The portable-C profile remains
  the long-term fallback for machines where that toolchain or its cost is not
  acceptable.
- Emulator timeouts must be checked visually as well as through serial and
  host verdict files; a modal AROS failure requester does not terminate
  FS-UAE.
