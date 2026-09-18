#ifndef AFSPLUS_DEBUG_OBSERVABILITY_H
#define AFSPLUS_DEBUG_OBSERVABILITY_H

/*
 * Draft development API. This is intentionally separate from the stable
 * on-disk format. Names and layouts are not frozen, except the trace event
 * codes of enum afsp_trace_event_code.
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
    AFSP_TRACE_ERROR      = UINT64_C(1) << 12,
    AFSP_TRACE_API        = UINT64_C(1) << 13,
    AFSP_TRACE_WINDOW     = UINT64_C(1) << 14,
    AFSP_TRACE_LIFECYCLE  = UINT64_C(1) << 15
};

/*
 * The event field of struct afsp_trace_event. Codes are stable: a new kind
 * takes the next number, and a retired code is never reused. Zero is none.
 */
enum afsp_trace_event_code {
    AFSP_EVENT_BEGIN = 1,
    AFSP_EVENT_DATA_WRITES_COMPLETE = 2,
    AFSP_EVENT_METADATA_DURABLE = 3,
    AFSP_EVENT_PUBLICATION_BEGIN = 4,
    AFSP_EVENT_CHECKPOINT_DURABLE = 5,
    AFSP_EVENT_ADOPTED = 6,
    AFSP_EVENT_FAILED = 7,
    AFSP_EVENT_API_BEGIN = 8,
    AFSP_EVENT_API_SUCCEEDED = 9,
    AFSP_EVENT_API_FAILED = 10,
    AFSP_EVENT_API_UNWOUND = 11,
    AFSP_EVENT_WINDOW_OPENED = 12,
    AFSP_EVENT_WINDOW_ATTACHED = 13,
    AFSP_EVENT_WINDOW_LOG_BEGIN = 14,
    AFSP_EVENT_WINDOW_LOG_DURABLE = 15,
    AFSP_EVENT_WINDOW_LOG_FAILED = 16,
    AFSP_EVENT_WINDOW_FAILED = 17,
    AFSP_EVENT_WINDOW_CLOSED = 18,
    AFSP_EVENT_WINDOW_DETACHED = 19,
    AFSP_EVENT_OBJECT_LOOKUP = 20,
    AFSP_EVENT_OBJECT_MAPPED = 21,
    AFSP_EVENT_OBJECT_MISSING = 22,
    AFSP_EVENT_ALLOCATION_BEGIN = 23,
    AFSP_EVENT_ALLOCATION_GRANTED = 24,
    AFSP_EVENT_ALLOCATION_RETIRED = 25,
    AFSP_EVENT_ALLOCATION_FAILED = 26,
    AFSP_EVENT_TREE_READ_BEGIN = 27,
    AFSP_EVENT_TREE_READ_COMPLETE = 28,
    AFSP_EVENT_TREE_SPILL_BEGIN = 29,
    AFSP_EVENT_TREE_SPILL_COMPLETE = 30,
    AFSP_EVENT_TREE_IO_FAILED = 31,
    AFSP_EVENT_RECLAIM_BEGIN = 32,
    AFSP_EVENT_RECLAIM_PROMOTED = 33,
    AFSP_EVENT_RECLAIM_BLOCKED = 34,
    AFSP_EVENT_RECLAIM_APPENDED = 35,
    AFSP_EVENT_RECLAIM_PLANNED = 36,
    AFSP_EVENT_RECLAIM_BUILT = 37,
    AFSP_EVENT_RECLAIM_FAILED = 38,
    AFSP_EVENT_MOUNT_BEGIN = 39,
    AFSP_EVENT_MOUNT_SELECTED = 40,
    AFSP_EVENT_MOUNT_INTENT_BEGIN = 41,
    AFSP_EVENT_MOUNT_INTENT_SCANNED = 42,
    AFSP_EVENT_MOUNT_INTENT_REPLAYED = 43,
    AFSP_EVENT_MOUNT_COMPLETE = 44,
    AFSP_EVENT_MOUNT_FAILED = 45,
    AFSP_EVENT_FORMAT_BEGIN = 46,
    AFSP_EVENT_FORMAT_METADATA_DURABLE = 47,
    AFSP_EVENT_FORMAT_PUBLICATION_BEGIN = 48,
    AFSP_EVENT_FORMAT_CHECKPOINT_DURABLE = 49,
    AFSP_EVENT_FORMAT_FAILED = 50,
    AFSP_EVENT_VERIFY_BEGIN = 51,
    AFSP_EVENT_VERIFY_PHASE = 52,
    AFSP_EVENT_VERIFY_FINDING = 53,
    AFSP_EVENT_VERIFY_COMPLETE = 54,
    AFSP_EVENT_VERIFY_FAILED = 55,
    AFSP_EVENT_DATA_WRITE_BEGIN = 56,
    AFSP_EVENT_DATA_WRITE_COMPLETE = 57,
    AFSP_EVENT_DATA_WRITE_FAILED = 58,
    AFSP_EVENT_INTENT_DATA_DURABLE = 59,
    AFSP_EVENT_INTENT_EMPTY_FLUSH = 60,
    AFSP_EVENT_VIEW_READ_BEGIN = 61,
    AFSP_EVENT_VIEW_READ_COMPLETE = 62,
    AFSP_EVENT_VIEW_READ_FAILED = 63,
    AFSP_EVENT_VIEW_MAINTENANCE_FAILED = 64
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

/*
 * Lightweight live device activity. This is separate from the detailed trace
 * stream: a volume with no sink installed pays no callback or clock cost.
 */
enum afsp_io_activity_operation {
    AFSP_IO_ACTIVITY_READ = 1,
    AFSP_IO_ACTIVITY_WRITE,
    AFSP_IO_ACTIVITY_FLUSH
};

enum afsp_io_activity_phase {
    AFSP_IO_ACTIVITY_BEGIN = 1,
    AFSP_IO_ACTIVITY_END
};

enum afsp_io_activity_flags {
    AFSP_IO_ACTIVITY_LBA_VALID = UINT32_C(1) << 0,
    AFSP_IO_ACTIVITY_SUCCESS = UINT32_C(1) << 1
};

enum afsp_io_activity_mask {
    AFSP_IO_ACTIVITY_MASK_READ = UINT32_C(1) << 0,
    AFSP_IO_ACTIVITY_MASK_WRITE = UINT32_C(1) << 1,
    AFSP_IO_ACTIVITY_MASK_FLUSH = UINT32_C(1) << 2
};

#define AFSP_IO_ACTIVITY_EVENT_VERSION UINT16_C(1)

struct afsp_io_activity_event {
    uint32_t size;
    uint16_t version;
    uint8_t operation;
    uint8_t phase;
    uint32_t flags;
    uint32_t reserved;
    uint64_t lba;
    uint64_t block_count;
};

typedef void (*afsp_io_activity_sink_fn)(
    void *ctx, const struct afsp_io_activity_event *event);

struct afsp_io_activity_sink {
    afsp_io_activity_sink_fn emit;
    void *ctx;
    uint32_t operation_mask;
};

#if defined(__STDC_VERSION__) && __STDC_VERSION__ >= 201112L
_Static_assert(sizeof(struct afsp_io_activity_event) == 32,
               "afsp_io_activity_event ABI drift");
#endif

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
int afsp_debug_set_io_activity_sink(
    void *volume, const struct afsp_io_activity_sink *sink);
int afsp_debug_set_check_level(void *volume, enum afsp_check_level level);
int afsp_debug_add_fault_rule(void *volume, const struct afsp_fault_rule *rule);
int afsp_debug_clear_fault_rules(void *volume);

#endif
