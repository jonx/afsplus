#ifndef LIBAFSPLUS_READER_H
#define LIBAFSPLUS_READER_H

/*
 * Minimal read-only profile.
 *
 * The implementation should be usable without threads and should permit
 * caller-supplied scratch buffers for constrained systems.
 */

#include <stdint.h>
#include <stddef.h>

struct afspr_scratch {
    void *buffer;
    size_t size;
};

struct afspr_entry {
    uint64_t object_id;
    uint64_t parent_id;
    uint64_t size;
    uint32_t type;
    const uint8_t *name;
    uint16_t name_len;
};

#endif
