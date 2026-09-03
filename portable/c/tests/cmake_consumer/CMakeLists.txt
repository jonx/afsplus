# SPDX-License-Identifier: BSD-2-Clause

cmake_minimum_required(VERSION 3.16)
project(afsplus_reader_consumer LANGUAGES C)

find_package(AFSPlusReader 0.1 CONFIG REQUIRED)
add_executable(afsplus-reader-consumer main.c)
target_link_libraries(afsplus-reader-consumer PRIVATE AFSPlus::reader)
