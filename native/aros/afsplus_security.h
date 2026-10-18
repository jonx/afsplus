/* SPDX-License-Identifier: BSD-2-Clause */
#ifndef AFSPLUS_AROS_SECURITY_H
#define AFSPLUS_AROS_SECURITY_H

#include <aros/libcall.h>
#include <libraries/security.h>

/* Public security.library ABI: GetTaskOwner is vector 7, Task* in D0,
 * returning uid<<16|gid. Some SDK distributions ship libraries/security.h
 * without its generated proto/inline headers. Bind that same ABI directly;
 * this is the sole authority path on every SDK. */
static inline ULONG afsplus_security_task_owner(struct Library *base,
    struct Task *task)
{
    return AROS_LC1(ULONG, secGetTaskOwner,
        AROS_LCA(struct Task *, task, D0), struct Library *, base, 7, Security);
}

#endif
