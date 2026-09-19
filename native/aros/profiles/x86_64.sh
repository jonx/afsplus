# SPDX-License-Identifier: BSD-2-Clause
#
# AROS on x86_64: the PC build, the one most people run. It is cross-built on
# macOS from the Homebrew LLVM, which knows the x86_64-unknown-aros triple,
# against the Developer directory of an x86_64 AROS nightly, and linked by a
# collect-aros built for x86_64.
#
# What this profile produces has been run: tools/check-qemu-aros-x86_64.sh
# mounts an AFS+ volume with it on a pc-x86_64 AROS in QEMU and puts the
# probes through it. The package says that, and says what is still untried.
#
# Sourced by tools/package-aros-dist.sh.

: "${LLVM:=/opt/homebrew/opt/llvm}"

aros_sdk=${AFSPLUS_AROS_SDK_ROOT:-$(ls -d "$HOME"/aros-native/AROS-*-linux-x86_64-system/Developer 2>/dev/null | tail -1)}

profile_id=${AFSPLUS_AROS_PROFILE_ID:-x86_64}
profile_qualified=${AFSPLUS_AROS_QUALIFIED:-qemu}
profile_cpu=${AFSPLUS_AROS_CPU:-x86_64}

aros_cc=${AFSPLUS_AROS_CC:-"$LLVM/bin/clang"}
aros_target=${AFSPLUS_AROS_TARGET:-x86_64-unknown-aros}
# The stock clang has no separate bare-metal codegen target to fall back to:
# it produces plain ELF for the aros triple and collect-aros does the rest.
aros_codegen_target=${AFSPLUS_AROS_CODEGEN_TARGET:-x86_64-unknown-aros}
# AROS builds x86_64 with the large code model; the ELF loader places a
# module's sections anywhere in the 64-bit address space, so a 32-bit
# displacement between them cannot be assumed. The red zone goes because AROS
# delivers exceptions and interrupts on the interrupted stack.
aros_arch_flags=${AFSPLUS_AROS_ARCH_FLAGS:--mcmodel=large -mno-red-zone}
# The stock clang knows the triple but not the platform, so the AROS macros
# that the MacAROS clang predefines have to be given here.
aros_defines=${AFSPLUS_AROS_DEFINES:--D__AROS__=1 -D__AROS=1 -DAROS=1 -DAMIGA=1 -D_AMIGA=1}
aros_codegen_defines=${AFSPLUS_AROS_CODEGEN_DEFINES:--D__AROS__}

# A nightly SDK has one include root: there is no separate gen tree.
aros_includes=${AFSPLUS_AROS_INCLUDES:-"-I $aros_sdk/include/aros/stdc -I $aros_sdk/include"}
aros_program_includes=${AFSPLUS_AROS_PROGRAM_INCLUDES:-"-isystem $aros_sdk/include/aros/stdc -isystem $aros_sdk/include/aros/posixc -isystem $aros_sdk/include"}
aros_glue_includes=${AFSPLUS_AROS_GLUE_INCLUDES:-"-I $aros_sdk/include -I $aros_sdk/include/aros/posixc -I $aros_sdk/include/aros/stdc"}

aros_lib_dirs=${AFSPLUS_AROS_LIB_DIRS:-"-L $aros_sdk/lib"}
aros_startup=${AFSPLUS_AROS_STARTUP:-"$aros_sdk/lib/startup.o"}
# No compiler-runtime library: the x86_64 code generator needs no builtin the
# AROS libraries do not already define. A link that did would fail loudly.
aros_module_libs=${AFSPLUS_AROS_MODULE_LIBS:---start-group -lstdc.static -lamiga -larossupport -lkeymap -lexpansion -lcommodities -licon -lintuition -lgadtools -llayers -laros -lpartition -liffparse -lgraphics -llocale -ldos -lutility -loop -llibinit -lautoinit -lposixc -lstdcio -lstdc -lexec -lpthread --end-group}
aros_program_libs=${AFSPLUS_AROS_PROGRAM_LIBS:---start-group -lpthread -lposixc -lstdc -lstdcio -ldos -lexec -laros -lautoinit -llibinit -lutility -lamiga -larossupport --end-group}

# collect-aros is AROS's linker wrapper and is built per CPU: the one a hosted
# aarch64 build makes is fixed to aarch64. Pkg's tools/build-aros-x86_64.sh
# builds the x86_64 one; this profile reuses it.
aros_collect_aros=${AFSPLUS_AROS_COLLECT_AROS:-"$HOME/aros-native/collect-aros-x86_64/collect-aros"}
# genmodule emits C and is the same host tool for every CPU.
aros_genmodule=${AFSPLUS_AROS_GENMODULE:-"$HOME/aros-build/bin/darwin-aarch64/tools/genmodule"}
aros_nm=${AFSPLUS_AROS_NM:-"$LLVM/bin/llvm-nm"}
aros_objdump=${AFSPLUS_AROS_OBJDUMP:-"$LLVM/bin/llvm-objdump"}

aros_rust_toolchain=${AFSPLUS_AROS_RUST_TOOLCHAIN:-nightly-2026-06-27}
aros_rust_target_json=${AFSPLUS_AROS_RUST_TARGET_JSON:-"$repo_root/native/aros/x86_64-unknown-aros.json"}
# SSE2 is the x86_64 baseline and AROS uses it; nothing beyond it is assumed,
# because an AROS PC may be any 64-bit machine.
aros_rust_cpu_features=${AFSPLUS_AROS_RUST_CPU_FEATURES-}
aros_platform_glue_dir=${AFSPLUS_AROS_PLATFORM_GLUE_DIR:-"${MACAROS_ROOT:-$repo_root/../Macaros}/hosted/rust"}

aros_abi_audit=${AFSPLUS_AROS_ABI_AUDIT:-"$repo_root/tools/check-aros-x86_64-abi.py"}
