# ADR-056: Qualify Alpha-0 and intent-log replay on native AROS/m68k

Status: Accepted

## Context

ADR-055 proved only that matching official AROS m68k media boot correctly in
FS-UAE. The next gate had to execute the real external AFS+ handler, not a host
model, and return the mutated image to the strict host checker. It also had to
revisit the experimental Rust/m68k toolchain rather than assuming that defects
observed on an older nightly still existed.

The first target JSON selected `M68020`. This was suitable for the first native
semantic gate, but not compatible with a plain 68000 Amiga 500. ADR-057 records
the subsequent plain-M68000 emulator qualification.

## Decision

[`tools/check-aros-m68k-alpha0-fsuae.sh`](../tools/check-aros-m68k-alpha0-fsuae.sh) is the reproducible native m68k
filesystem gate. Given a matching official boot ADF and system ISO, it:

1. builds `afsplus-aros-ffi` and its AROS `std` with the qualified patched
   nightly, release O2, no LTO and one codegen unit;
2. generates the handler entry, compiles the C packet/trackdisk shell and links
   the final module directly with `collect-aros`;
3. rejects undefined symbols and diagnostic trace strings in that module;
4. boots a fresh case-insensitive image under native AROS in FS-UAE, verifies
   that the DOS device and volume route to the same handler port, then runs
   create/read/write/sparse-write/truncate/rename/two flushes/case-folded
   lookup/case-only rename/reopen/readback;
5. returns the image and requires a clean schema-5 host checker with no pending
   intent-log record; and
6. generates and boots all six deterministic crash fixtures, requiring the
   four `old` and two `new` outcomes, their exact generation, a clean checker
   and no pending log record.

The command extracts the ISO into a private work directory and installs only
the external handler, DOSDriver and two target probes there. It does not modify
an AROS or MacAROS source tree. Its evidence records the compiler, target,
handler, boot-media and ROM hashes and explicitly makes no physical-A500 or
plain-68000 claim.

The accepted run used FS-UAE 3.2.35 with the `A4000/040` profile. The Alpha-0
image advanced from generation 1 to generation 7 and retained generations 7/6.
All six recovery cases passed: cuts 0 through 3 remained at the old generation
2 state, while cuts 4 and 5 replayed to generation 3. Every final image was
clean and had zero pending log records.

## Rust/LLVM qualification

The compiler is the 2026-08-30 nightly source (`fd7ed57df`) with LLVM PR
[#168485](https://github.com/llvm/llvm-project/pull/168485) applied. That patch
resolves the live-CCR `COPY` miscompile reported as LLVM
[#152816](https://github.com/llvm/llvm-project/issues/152816). LLVM
[#213564](https://github.com/llvm/llvm-project/issues/213564) remains open
because the accepted CCR save/restore mechanism is a temporary solution and
has an explicit 68000-versus-68010+ compatibility cost.

The runtime investigation exposed a separate m68k code-generation defect: an
out-of-line `Result<(), CoreError>` return overwrote its indirect result slot
with the return address. The valid allocation-region record was consequently
interpreted as an error and the handler stopped inside error destruction.
This is not attributed to the CCR tickets. AFS+ avoids the defective large
success-return ABI on its hot validator by returning a scalar validation enum
and constructing the owned diagnostic only in the caller's cold error path.
The host tests retain the same diagnostics and the native operation/replay
gates prove both success paths.

The standard-library PAL was rebased outside this repository for the current
nightly allocation, I/O-error and unsupported m68k subsystem APIs. That PAL and
patched compiler remain an experimental build prerequisite, not part of the
AFS+ disk format or VFS contract.

Formatting reductions are confined to the m68k build: modern targets retain
the complete contextual mount and `FormatError` diagnostics. The classic path
uses bounded checkpoint-status formatting and may return the underlying state
error without the additional formatted checkpoint wrapper.

## Consequences

- Native AROS/m68k handler semantics and modeled recovery are now qualified on
  an M68020-or-newer emulator profile.
- This closes the M68020-or-newer filesystem part of validation stage 3.
- ADR-057 adds plain-M68000 code generation, instruction auditing and the same
  operation/replay matrix on an A500-configured emulator. Memory-budget,
  performance and physical-machine claims remain open.
- Compiler workarounds must stay small, architecture-neutral where practical,
  and backed by native runtime evidence; they must not change on-disk semantics.
