# SPDX-License-Identifier: BSD-2-Clause

cmake_minimum_required(VERSION 3.16)
project(afsplus_portable_c VERSION 0.1.0 LANGUAGES C)

option(AFSPLUS_READER_BUILD_EXAMPLE "Build the host-file probe example" ON)
option(AFSPLUS_READER_BUILD_FUZZ_TOOLS "Build corpus and replay tools" OFF)

add_library(afsplus_reader STATIC reader.c)
add_library(AFSPlus::reader ALIAS afsplus_reader)
target_compile_features(afsplus_reader PUBLIC c_std_99)
set_target_properties(afsplus_reader PROPERTIES
    C_EXTENSIONS OFF
    EXPORT_NAME reader
    PUBLIC_HEADER "${CMAKE_CURRENT_SOURCE_DIR}/../../api/libafsplus_reader.h"
)
target_include_directories(afsplus_reader
    PUBLIC
        "$<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/../../api>"
        "$<INSTALL_INTERFACE:include>"
    PRIVATE
        "${CMAKE_CURRENT_SOURCE_DIR}/../../spec"
)

if(AFSPLUS_READER_BUILD_EXAMPLE)
    add_executable(afsplus-reader-probe examples/probe_file.c)
    target_link_libraries(afsplus-reader-probe PRIVATE AFSPlus::reader)
    set_target_properties(afsplus-reader-probe PROPERTIES C_EXTENSIONS OFF)
endif()

if(AFSPLUS_READER_BUILD_FUZZ_TOOLS)
    add_library(afsplus_reader_fuzz_harness STATIC fuzz/harness.c)
    target_link_libraries(afsplus_reader_fuzz_harness PUBLIC AFSPlus::reader)
    target_include_directories(afsplus_reader_fuzz_harness
        PUBLIC "${CMAKE_CURRENT_SOURCE_DIR}/fuzz")
    set_target_properties(afsplus_reader_fuzz_harness PROPERTIES
        C_EXTENSIONS OFF)

    add_executable(afsplus-fuzz-pack fuzz/seed_pack.c)
    target_link_libraries(afsplus-fuzz-pack
        PRIVATE afsplus_reader_fuzz_harness)
    set_target_properties(afsplus-fuzz-pack PROPERTIES C_EXTENSIONS OFF)

    add_executable(afsplus-fuzz-replay fuzz/replay.c)
    target_link_libraries(afsplus-fuzz-replay
        PRIVATE afsplus_reader_fuzz_harness)
    set_target_properties(afsplus-fuzz-replay PROPERTIES C_EXTENSIONS OFF)

    add_executable(afsplus-fuzz-smoke fuzz/fuzz_target.c fuzz/smoke.c)
    target_link_libraries(afsplus-fuzz-smoke
        PRIVATE afsplus_reader_fuzz_harness)
    set_target_properties(afsplus-fuzz-smoke PROPERTIES C_EXTENSIONS OFF)
endif()

include(GNUInstallDirs)
include(CMakePackageConfigHelpers)
configure_package_config_file(
    "${CMAKE_CURRENT_SOURCE_DIR}/cmake/AFSPlusReaderConfig.cmake.in"
    "${CMAKE_CURRENT_BINARY_DIR}/AFSPlusReaderConfig.cmake"
    INSTALL_DESTINATION "${CMAKE_INSTALL_LIBDIR}/cmake/AFSPlusReader"
)
write_basic_package_version_file(
    "${CMAKE_CURRENT_BINARY_DIR}/AFSPlusReaderConfigVersion.cmake"
    VERSION "${PROJECT_VERSION}"
    COMPATIBILITY SameMajorVersion
)
install(TARGETS afsplus_reader
    EXPORT AFSPlusReaderTargets
    ARCHIVE DESTINATION "${CMAKE_INSTALL_LIBDIR}"
    PUBLIC_HEADER DESTINATION "${CMAKE_INSTALL_INCLUDEDIR}"
)
install(EXPORT AFSPlusReaderTargets
    NAMESPACE AFSPlus::
    DESTINATION "${CMAKE_INSTALL_LIBDIR}/cmake/AFSPlusReader"
)
install(FILES
    "${CMAKE_CURRENT_BINARY_DIR}/AFSPlusReaderConfig.cmake"
    "${CMAKE_CURRENT_BINARY_DIR}/AFSPlusReaderConfigVersion.cmake"
    DESTINATION "${CMAKE_INSTALL_LIBDIR}/cmake/AFSPlusReader"
)
