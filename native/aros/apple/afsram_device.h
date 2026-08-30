/* SPDX-License-Identifier: BSD-2-Clause */

#ifndef AFSPLUS_AROS_APPLE_AFSRAM_DEVICE_H
#define AFSPLUS_AROS_APPLE_AFSRAM_DEVICE_H

#include <exec/devices.h>
#include <exec/execbase.h>
#include <exec/types.h>

#include <stdint.h>

struct AfsRamBase {
    struct Device device;
    struct Unit unit;
    struct ExecBase *sys_base;
    APTR kernel_base;
    uintptr_t payload_start;
    uint64_t payload_size;
    BOOL ready;
};

#endif /* AFSPLUS_AROS_APPLE_AFSRAM_DEVICE_H */
