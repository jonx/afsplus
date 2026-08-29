#ifndef AFSPLUS_PERFORMANCE_HINTS_H
#define AFSPLUS_PERFORMANCE_HINTS_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef enum afsplus_access_hint {
    AFSPLUS_HINT_NORMAL = 0,
    AFSPLUS_HINT_SEQUENTIAL,
    AFSPLUS_HINT_RANDOM,
    AFSPLUS_HINT_WILL_NEED,
    AFSPLUS_HINT_DONT_NEED,
    AFSPLUS_HINT_NO_REUSE,
    AFSPLUS_HINT_LATENCY_SENSITIVE,
    AFSPLUS_HINT_BULK_THROUGHPUT,
    AFSPLUS_HINT_MMAP_EXPECTED,
    AFSPLUS_HINT_DIRECT_IO_PREFERRED,
    AFSPLUS_HINT_TEMPORARY,
    AFSPLUS_HINT_IMMUTABLE_EXPECTED
} afsplus_access_hint_t;

typedef enum afsplus_preallocate_mode {
    AFSPLUS_PREALLOC_RESERVE = 0,
    AFSPLUS_PREALLOC_ENSURE_ALLOCATED = 1,
    AFSPLUS_PREALLOC_CONTIGUOUS_PREFERRED = 2
} afsplus_preallocate_mode_t;

typedef struct afsplus_placement_hint {
    uint64_t preferred_extent_bytes;
    uint64_t alignment_bytes;
    uint32_t flags;
    uint32_t reserved;
} afsplus_placement_hint_t;

/* Draft filesystem-neutral API shapes. Exact ABI is not frozen. */
int afsplus_advise_range(void *handle,
                         uint64_t offset,
                         uint64_t length,
                         afsplus_access_hint_t hint);

int afsplus_preallocate(void *handle,
                        uint64_t offset,
                        uint64_t length,
                        afsplus_preallocate_mode_t mode,
                        const afsplus_placement_hint_t *placement);

int afsplus_prefetch_range(void *handle,
                           uint64_t offset,
                           uint64_t length);

int afsplus_seal_content(void *handle, uint32_t flags);

#ifdef __cplusplus
}
#endif

#endif
