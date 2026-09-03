/* SPDX-License-Identifier: BSD-2-Clause */
#ifndef AFSPLUS_READER_INTERNAL_H
#define AFSPLUS_READER_INTERNAL_H

/* Private cooperation surface between reader.c and writer.c. */

#include "libafsplus_reader.h"

#define AFSPR_INTERNAL_NAMESPACE_SCAN 1u
#define AFSPR_INTERNAL_NAMESPACE_SOURCE 2u
#define AFSPR_INTERNAL_NAMESPACE_TARGET 3u

int afspr_internal_validate_object_watermark(
    const struct afspr_block_ops *ops, const struct afspr_scratch *scratch,
    const struct afspr_probe_result *volume, uint64_t next_object_id,
    struct afspr_diagnostic *diagnostic, size_t diagnostic_size);

/* `view` must come from a successful immediately preceding log scan. */
int afspr_internal_intent_file_size(
    const struct afspr_block_ops *ops, const struct afspr_scratch *scratch,
    const struct afspr_probe_result *volume,
    const struct afspr_intent_view *view, uint64_t object_id,
    uint64_t *size_bytes, struct afspr_diagnostic *diagnostic,
    size_t diagnostic_size);

/*
 * Select one committed-free block while reserving the data extents named by
 * the validated intent prefix. `view` must come from a successful immediately
 * preceding log scan under writer serialization. The returned block remains
 * FREE in the base checkpoint; durable ownership begins only when the caller
 * publishes a log record naming it.
 */
int afspr_internal_find_log_data_block(
    const struct afspr_block_ops *ops, const struct afspr_scratch *scratch,
    const struct afspr_probe_result *volume,
    const struct afspr_intent_view *view, uint64_t free_block_floor,
    uint64_t *data_block, struct afspr_diagnostic *diagnostic,
    size_t diagnostic_size);

int afspr_internal_preflight_file_namespace(
    const struct afspr_block_ops *ops, const struct afspr_scratch *scratch,
    const struct afspr_probe_result *volume, uint64_t source_parent_id,
    const void *source_name, size_t source_name_len,
    uint64_t target_parent_id, const void *target_name,
    size_t target_name_len, int source_must_exist, int inspect_target,
    struct afspr_intent_view *view, int *destination_conflict,
    int *destination_is_file, uint64_t *next_object_id, uint32_t *phase,
    struct afspr_diagnostic *diagnostic, size_t diagnostic_size);

#endif
