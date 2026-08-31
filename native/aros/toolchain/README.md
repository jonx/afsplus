# Experimental Rust/m68k toolchain deltas

These patches record the AFS+ plain-68000 qualification deltas that are not
part of the filesystem implementation or disk format. They apply to the
2026-08-30 Rust nightly source at `fd7ed57df`, after LLVM PR 168485 and the
AROS standard-library PAL have been rebased onto that source.

`llvm-m68k-m68000-codegen.patch` fixes two independently reproduced backend
problems:

- an external tail call could become a short `BRA` that cannot represent the
  final link distance on M68000;
- an overflow-producing 32-bit multiply, especially the
  `checked_mul(constant).unwrap_or(...)` shape used by bitmap decoding, could
  select M68020 `MULU.L`/`MULS.L` instructions for `-mcpu=M68000`.

The LLVM patch includes M68000 and M68020 code-generation tests. The long
multiply instruction definitions are also guarded by the M68020 predicate so
an unqualified instruction cannot silently re-enter another lowering path.

`cargo-build-std-no-proc-macro.patch` is a temporary patch for a dedicated
host Cargo binary. Cargo otherwise adds `proc_macro` implicitly whenever
`-Zbuild-std=std` is requested, but the experimental AROS target does not
provide the dynamic-loading support needed by that crate. Do not replace the
normal host Cargo with this binary.

From the Rust source root:

```sh
patch -p1 < /path/to/afsplus/native/aros/toolchain/llvm-m68k-m68000-codegen.patch
patch -p1 < /path/to/afsplus/native/aros/toolchain/cargo-build-std-no-proc-macro.patch
```

After rebuilding/installing LLVM and the stage-1 compiler, run both new LLVM
tests for `M68000` and `M68020`. The AFS+ gate additionally disassembles the
linked handler and rejects every long-multiply encoding before FS-UAE starts.

On macOS, pass the installed LLVM library directory through
`AFSPLUS_AROS_M68K_LLVM_LIB`, not `DYLD_LIBRARY_PATH` around the shell script.
System Integrity Protection removes `DYLD_*` while launching Apple's
`/bin/sh`; the gate exports it internally after validating `libLLVM.dylib`.

Example qualification inputs:

```sh
AFSPLUS_AROS_M68K_LLVM_LIB=/path/to/llvm/install/lib \
AFSPLUS_AROS_M68K_CARGO=/path/to/dedicated-cargo \
AFSPLUS_AROS_M68K_RUST_TARGET_JSON=native/aros/m68k-unknown-aros-m68000.json \
AFSPLUS_AROS_M68K_MODEL=A500 \
AFSPLUS_AROS_M68K_CPU_SPEED=max \
AFSPLUS_AROS_M68K_FAST_MEMORY=8192 \
AFSPLUS_AROS_M68K_ZORRO_III_MEMORY=0 \
AFSPLUS_AROS_M68K_BOOT_ADF=/path/to/Emergency-Boot.adf \
AFSPLUS_AROS_M68K_SYSTEM_ISO=/path/to/aros-amiga-m68k.iso \
    tools/check-aros-m68k-alpha0-fsuae.sh
```

The emulator result is a plain-68000 functional/recovery claim only. It is not
a physical-Amiga, memory-budget, or performance qualification.
