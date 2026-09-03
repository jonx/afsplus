/* SPDX-License-Identifier: BSD-2-Clause */
#ifndef AFSPLUS_READER_INTERNAL_H
#define AFSPLUS_READER_INTERNAL_H

/* Private cooperation surface between reader.c and writer.c. */

#include "libafsplus_reader.h"

#define AFSPR_INTERNAL_RENAME_SCAN 1u
#define AFSPR_INTERNAL_RENAME_SOURCE 2u
#define AFSPR_INTERNAL_RENAME_TARGET 3u

int afspr_internal_preflight_rename_no_replace(
    const struct afspr_block_ops *ops, const struct afspr_scratch *scratch,
    const struct afspr_probe_result *volume, uint64_t source_parent_id,
    const void *source_name, size_t source_name_len,
    uint64_t target_parent_id, const void *target_name,
    size_t target_name_len, struct afspr_intent_view *view,
    int *destination_conflict, uint32_t *phase,
    struct afspr_diagnostic *diagnostic, size_t diagnostic_size);

#endif
