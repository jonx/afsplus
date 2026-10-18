/* SPDX-License-Identifier: BSD-2-Clause */
#ifndef AFSPLUS_AROS_SECURITY_ABI_H
#define AFSPLUS_AROS_SECURITY_ABI_H

#include <stdint.h>

/* ADR-134: permanent loaded service; no compatible revision may expunge it. */
#define AFSPLUS_SECURITY_VERSION 45
#define AFSPLUS_SECURITY_REVISION 17
#define AFSPLUS_SECURITY_ROOT_UID UINT32_C(65535)
#define AFSPLUS_SECURITY_GET_TASK_OWNER_LVO 7

#endif
