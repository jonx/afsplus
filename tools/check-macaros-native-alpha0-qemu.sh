#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause

set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
AFSPLUS_MACAROS_QEMU_MODE=alpha0 \
AFSPLUS_MACAROS_QEMU_EXTRACT=${AFSPLUS_MACAROS_QEMU_EXTRACT:-1} \
AFSPLUS_MACAROS_QEMU_OUTPUT=${AFSPLUS_MACAROS_QEMU_OUTPUT:-"$repo_root/build/macaros-native-alpha0-qemu"} \
    exec "$repo_root/tools/check-macaros-native-block-qemu.sh"
