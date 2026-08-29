#ifndef AFSPLUS_SECURITY_ACL_H
#define AFSPLUS_SECURITY_ACL_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct afsp_uuid128 {
    uint8_t bytes[16];
} afsp_uuid128;

typedef enum afsp_principal_kind {
    AFSP_PRINCIPAL_WELL_KNOWN = 1,
    AFSP_PRINCIPAL_REALM_ID   = 2
} afsp_principal_kind;

typedef enum afsp_well_known_principal {
    AFSP_WKP_OWNER = 1,
    AFSP_WKP_GROUP,
    AFSP_WKP_EVERYONE,
    AFSP_WKP_AUTHENTICATED,
    AFSP_WKP_ANONYMOUS,
    AFSP_WKP_SYSTEM
} afsp_well_known_principal;

typedef struct afsp_principal_ref {
    uint32_t kind;
    uint32_t well_known;
    afsp_uuid128 realm_uuid;
    afsp_uuid128 principal_uuid;
} afsp_principal_ref;

typedef enum afsp_ace_type {
    AFSP_ACE_ALLOW = 1,
    AFSP_ACE_DENY  = 2,
    AFSP_ACE_AUDIT = 3
} afsp_ace_type;

enum {
    AFSP_RIGHT_READ_DATA        = 1ull << 0,
    AFSP_RIGHT_WRITE_DATA       = 1ull << 1,
    AFSP_RIGHT_APPEND_DATA      = 1ull << 2,
    AFSP_RIGHT_EXECUTE          = 1ull << 3,
    AFSP_RIGHT_READ_ATTRIBUTES  = 1ull << 4,
    AFSP_RIGHT_WRITE_ATTRIBUTES = 1ull << 5,
    AFSP_RIGHT_READ_XATTR       = 1ull << 6,
    AFSP_RIGHT_WRITE_XATTR      = 1ull << 7,
    AFSP_RIGHT_DELETE           = 1ull << 8,
    AFSP_RIGHT_DELETE_CHILD     = 1ull << 9,
    AFSP_RIGHT_READ_ACL         = 1ull << 10,
    AFSP_RIGHT_WRITE_ACL        = 1ull << 11,
    AFSP_RIGHT_WRITE_OWNER      = 1ull << 12
};

enum {
    AFSP_ACE_INHERIT_FILE      = 1u << 0,
    AFSP_ACE_INHERIT_DIRECTORY = 1u << 1,
    AFSP_ACE_INHERIT_ONLY      = 1u << 2,
    AFSP_ACE_NO_PROPAGATE      = 1u << 3,
    AFSP_ACE_INHERITED         = 1u << 4,
    AFSP_ACE_AUDIT_SUCCESS     = 1u << 5,
    AFSP_ACE_AUDIT_FAILURE     = 1u << 6
};

typedef struct afsp_ace {
    uint32_t type;
    uint32_t flags;
    uint64_t rights;
    afsp_principal_ref principal;
} afsp_ace;

typedef struct afsp_security_descriptor_view {
    uint64_t descriptor_id;
    afsp_principal_ref owner;
    afsp_principal_ref owning_group;
    uint32_t control_flags;
    uint32_t ace_count;
    const afsp_ace *aces;
} afsp_security_descriptor_view;

typedef enum afsp_security_fidelity {
    AFSP_SECURITY_FULL = 0,
    AFSP_SECURITY_PRESERVED_NOT_FULLY_EXPOSED = 1,
    AFSP_SECURITY_DEGRADED = 2
} afsp_security_fidelity;

typedef struct afsp_security_capabilities {
    uint32_t canonical_acl;
    uint32_t allow_deny;
    uint32_t inheritance;
    uint32_t audit_acl;
    uint32_t principal_mapping_complete;
    uint32_t host_enforcement_full;
    uint32_t fidelity;
} afsp_security_capabilities;

/* Draft semantic API. Concrete ABI/versioning is not frozen. */
int afsp_get_security_descriptor(uint64_t object_id,
                                 afsp_security_descriptor_view *out_desc);
int afsp_set_security_descriptor(uint64_t object_id,
                                 const afsp_security_descriptor_view *desc,
                                 uint32_t flags);
int afsp_check_access(uint64_t object_id,
                      const afsp_principal_ref *subject,
                      uint64_t requested_rights,
                      uint64_t *granted_rights);
int afsp_get_security_capabilities(afsp_security_capabilities *out_caps);

#ifdef __cplusplus
}
#endif

#endif
