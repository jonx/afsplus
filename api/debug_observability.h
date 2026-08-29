#ifndef AFSPLUS_DEBUG_OBSERVABILITY_H
#define AFSPLUS_DEBUG_OBSERVABILITY_H

/*
 * Draft development API. This is intentionally separate from the stable
 * on-disk format. Names and layouts are not frozen.
 */

#include <stdint.h>
#include <stddef.h>

typedef uint64_t afsp_trace_seq;
typedef uint64_t afsp_txid;
typedef uint64_t afsp_object_id;

enum afsp_trace_category {
    AFSP_TRACE_TX         = UINT64_C(1) << 0,
    AFSP_TRACE_CHECKPOINT = UINT64_C(1) << 1,
    AFSP_TRACE_IO         = UINT64_C(1) << 2,
    AFSP_TRACE_ALLOC      = UINT64_C(1) << 3,
    AFSP_TRACE_CACHE      = UINT64_C(1) << 4,
    AFSP_TRACE_BTREE      = UINT64_C(1) << 5,
    AFSP_TRACE_OBJECT     = UINT64_C(1) << 6,
    AFSP_TRACE_DIRECTORY  = UINT64_C(1) << 7,
    AFSP_TRACE_CATALOG    = UINT64_C(1) << 8,
    AFSP_TRACE_CHANGE     = UINT64_C(1) << 9,
    AFSP_TRACE_RECLAIM    = UINT64_C(1) << 10,
    AFSP_TRACE_REPAIR     = UINT64_C(1) << 11,
    AFSP_TRACE_ERROR      = UINT64_C(1) << 12
};

struct afsp_trace_event {
    afsp_trace_seq sequence;
    uint64_t timestamp;
    afsp_txid transaction_id;
    afsp_object_id object_id;
    uint64_t block;
    uint64_t arg0;
    uint64_t arg1;
    uint32_t task_id;
    uint16_t category;
    uint16_t event;
};

typedef void (*afsp_trace_sink_fn)(void *ctx,
                                   const struct afsp_trace_event *event);

struct afsp_trace_sink {
    afsp_trace_sink_fn emit;
    void *ctx;
    uint64_t category_mask;
};

enum afsp_check_level {
    AFSP_CHECK_ALWAYS = 0,
    AFSP_CHECK_DEBUG,
    AFSP_CHECK_PARANOID,
    AFSP_CHECK_FULL
};

enum afsp_fault_kind {
    AFSP_FAULT_READ,
    AFSP_FAULT_WRITE,
    AFSP_FAULT_TORN_WRITE,
    AFSP_FAULT_FLUSH,
    AFSP_FAULT_ENOSPC,
    AFSP_FAULT_ALLOC_MEMORY,
    AFSP_FAULT_FORCE_EVICT,
    AFSP_FAULT_CORRUPT_AFTER_WRITE,
    AFSP_FAULT_POWER_CUT
};

struct afsp_fault_rule {
    enum afsp_fault_kind kind;
    uint64_t nth;
    uint64_t event_code;
    uint64_t arg0;
    uint64_t arg1;
};

/* Proposed debug/introspection operations. */
int afsp_debug_set_trace_sink(void *volume, const struct afsp_trace_sink *sink);
int afsp_debug_set_check_level(void *volume, enum afsp_check_level level);
int afsp_debug_add_fault_rule(void *volume, const struct afsp_fault_rule *rule);
int afsp_debug_clear_fault_rules(void *volume);

#endif
