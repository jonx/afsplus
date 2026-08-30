#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause

# Add a shared file-backed RAM object to an otherwise standard QEMU command.
# The caller selects it with QEMU_MACHINE=...,memory-backend=afsplus-memory.

set -eu

: "${AFSPLUS_QEMU_MEMORY_FILE:?set AFSPLUS_QEMU_MEMORY_FILE}"
qemu=${AFSPLUS_QEMU_REAL_BIN:-qemu-system-aarch64}

[ ! -e "$AFSPLUS_QEMU_MEMORY_FILE" ] || {
    echo "Refusing to replace guest-memory image: $AFSPLUS_QEMU_MEMORY_FILE" >&2
    exit 73
}

exec "$qemu" "$@" \
    -object "memory-backend-file,id=afsplus-memory,size=512M,mem-path=$AFSPLUS_QEMU_MEMORY_FILE,share=on"
