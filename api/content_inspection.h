#ifndef AFSPLUS_CONTENT_INSPECTION_H
#define AFSPLUS_CONTENT_INSPECTION_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef uint64_t afsp_object_id_t;
typedef uint64_t afsp_content_generation_t;
typedef uint64_t afsp_change_sequence_t;
typedef uint64_t afsp_security_subscription_t;
typedef uint64_t afsp_generation_handle_t;

typedef enum afsp_security_event_kind {
    AFSP_SEC_CONTENT_CREATED = 1,
    AFSP_SEC_CONTENT_GENERATION_CHANGED = 2,
    AFSP_SEC_OBJECT_CLONED = 3,
    AFSP_SEC_OBJECT_DELETED = 4,
    AFSP_SEC_OBJECT_RENAMED = 5,
    AFSP_SEC_EXECUTABLE_INTENT = 6,
    AFSP_SEC_SECURITY_METADATA_CHANGED = 7,
    AFSP_SEC_ORIGIN_METADATA_CHANGED = 8
} afsp_security_event_kind_t;

typedef enum afsp_security_authorization {
    AFSP_SEC_AUTH_ALLOW = 1,
    AFSP_SEC_AUTH_DENY = 2,
    AFSP_SEC_AUTH_DEFER = 3
} afsp_security_authorization_t;

typedef struct afsp_security_event {
    uint32_t struct_version;
    uint32_t kind;
    uint8_t filesystem_uuid[16];
    afsp_object_id_t object_id;
    afsp_content_generation_t old_content_generation;
    afsp_content_generation_t new_content_generation;
    afsp_change_sequence_t change_sequence;
    uint64_t checkpoint_generation;
    uint64_t logical_size;
    uint64_t flags;
} afsp_security_event_t;

typedef struct afsp_security_subscription_options {
    uint32_t struct_version;
    uint32_t flags;
    afsp_change_sequence_t start_sequence;
    uint64_t event_mask;
    uint64_t object_type_mask;
    uint64_t minimum_size;
    uint64_t maximum_size;
} afsp_security_subscription_options_t;

/* Draft API. Exact ABI is not frozen. */
int afsp_security_subscribe(const afsp_security_subscription_options_t *options,
                            afsp_security_subscription_t *out_subscription);

int afsp_security_next(afsp_security_subscription_t subscription,
                       afsp_security_event_t *events,
                       size_t event_capacity,
                       size_t *out_event_count,
                       afsp_change_sequence_t *out_next_sequence);

int afsp_security_open_generation(afsp_object_id_t object_id,
                                  afsp_content_generation_t generation,
                                  afsp_generation_handle_t *out_handle);

int afsp_security_read_generation(afsp_generation_handle_t handle,
                                  uint64_t offset,
                                  void *buffer,
                                  size_t length,
                                  size_t *out_read);

int afsp_security_close_generation(afsp_generation_handle_t handle);

/* OS security layer may expose permission events through this contract. */
int afsp_security_authorize(uint64_t authorization_request_id,
                            afsp_security_authorization_t decision);

#ifdef __cplusplus
}
#endif

#endif
