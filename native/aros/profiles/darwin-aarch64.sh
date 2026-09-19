# SPDX-License-Identifier: BSD-2-Clause
#
# Hosted MacAROS on Apple Silicon: the qualified profile. The handler built
# here is the one the Hosted gates run, so this profile's package is the only
# one that carries a runtime claim.
#
# Sourced by tools/package-aros-dist.sh. Every value may be overridden from
# the environment, because a machine may keep its AROS build elsewhere.

: "${AROS_BUILD:=$HOME/aros-build}"
: "${AROS_CROSSTOOLS:=$HOME/aros-crosstools}"

aros_sdk=${AFSPLUS_AROS_SDK_ROOT:-"$AROS_BUILD/bin/darwin-aarch64"}
aros_developer="$aros_sdk/AROS/Developer"

profile_id=${AFSPLUS_AROS_PROFILE_ID:-darwin-aarch64}
# Hosted MacAROS runs this handler in the Alpha-0, DOS, crash-replay, S1, S2
# and S3 gates; that is what "qualified" means here.
profile_qualified=${AFSPLUS_AROS_QUALIFIED:-yes}
profile_cpu=${AFSPLUS_AROS_CPU:-aarch64}

aros_cc=${AFSPLUS_AROS_CC:-"$AROS_CROSSTOOLS/bin/clang"}
aros_target=${AFSPLUS_AROS_TARGET:-aarch64-unknown-aros}
aros_codegen_target=${AFSPLUS_AROS_CODEGEN_TARGET:-aarch64-unknown-none-elf}
# Darwin may alter x18 across host signal delivery, so Hosted reserves it in
# every object. ADR-051 records that this is a Hosted requirement, not an AFS+
# one.
aros_arch_flags=${AFSPLUS_AROS_ARCH_FLAGS:--mcmodel=large -ffixed-x18}
# The MacAROS clang predefines the AROS platform macros for this triple.
aros_defines=${AFSPLUS_AROS_DEFINES:-}
aros_codegen_defines=${AFSPLUS_AROS_CODEGEN_DEFINES:--D__arm64__ -D__AROS__}

aros_includes=${AFSPLUS_AROS_INCLUDES:-"-I $aros_developer/include/aros/stdc -I $aros_developer/include -I $aros_sdk/gen/include"}
aros_program_includes=${AFSPLUS_AROS_PROGRAM_INCLUDES:-"-isystem $aros_developer/include/aros/stdc -isystem $aros_developer/include -isystem $aros_sdk/gen/include -isystem $aros_sdk/gen/include/aros/posixc"}
aros_glue_includes=${AFSPLUS_AROS_GLUE_INCLUDES:-"-I $aros_sdk/gen/include -I $aros_developer/include -I $aros_sdk/gen/include/aros/posixc -I $aros_developer/include/aros/stdc"}

aros_lib_dirs=${AFSPLUS_AROS_LIB_DIRS:-"-L $aros_developer/lib -L $AROS_CROSSTOOLS/lib/generic"}
aros_startup=${AFSPLUS_AROS_STARTUP:-"$aros_developer/lib/startup.o"}
aros_module_libs=${AFSPLUS_AROS_MODULE_LIBS:---start-group -lstdc.static -lmui -lamiga -larossupport -lamiga -lcodesets -lkeymap -lexpansion -lcommodities -ldiskfont -lasl -lmuimaster -ldatatypes -lcybergraphics -lworkbench -licon -lintuition -lgadtools -llayers -laros -lpartition -liffparse -lgraphics -llocale -ldos -lutility -loop -llibinit -lautoinit -lposixc -lstdcio -lstdc -lexec -lpthread -lclang_rt.builtins-aarch64 --end-group}
aros_program_libs=${AFSPLUS_AROS_PROGRAM_LIBS:---start-group -lpthread -lposixc -lstdc -lstdcio -ldos -lexec -laros -lautoinit -llibinit -lutility -lamiga -larossupport --end-group -lclang_rt.builtins-aarch64}

aros_collect_aros=${AFSPLUS_AROS_COLLECT_AROS:-"$aros_sdk/tools/collect-aros"}
aros_genmodule=${AFSPLUS_AROS_GENMODULE:-"$aros_sdk/tools/genmodule"}
aros_nm=${AFSPLUS_AROS_NM:-"$AROS_CROSSTOOLS/bin/llvm-nm"}
aros_objdump=${AFSPLUS_AROS_OBJDUMP:-"$AROS_CROSSTOOLS/bin/llvm-objdump"}

aros_rust_toolchain=${AFSPLUS_AROS_RUST_TOOLCHAIN:-nightly-2026-06-27}
aros_rust_target_json=${AFSPLUS_AROS_RUST_TARGET_JSON:-"${MACAROS_ROOT:-$repo_root/../Macaros}/hosted/rust/aarch64-unknown-aros.json"}
# Every Apple Silicon CPU has the ARMv8 CRC32 instructions.
aros_rust_cpu_features=${AFSPLUS_AROS_RUST_CPU_FEATURES-+crc}
aros_platform_glue_dir=${AFSPLUS_AROS_PLATFORM_GLUE_DIR:-"${MACAROS_ROOT:-$repo_root/../Macaros}/hosted/rust"}

aros_abi_audit=${AFSPLUS_AROS_ABI_AUDIT:-"$repo_root/tools/check-aros-aarch64-abi.py"}
