/* SPDX-License-Identifier: BSD-2-Clause */
#ifndef AFSPLUS_AROS_AUTHORITY_H
#define AFSPLUS_AROS_AUTHORITY_H

/* Never enter the DOS library loader from its own filesystem task. Borrow an
 * already loaded library under Exec's library-list exclusion, pin it while
 * the real Open vector may wait on its semaphore. Drop the temporary pin
 * after OpenLibrary has acquired its own reference, then use normal Close.
 * A failed Open fails closed and preserves any pending expunge request. */
static int32_t afsplus_aros_authorize_policy(struct ExecBase *SysBase,
    const struct DosPacket *packet, uint32_t policy)
{
    struct Library *secBase;
    struct Library *library;
    ULONG owner;
    struct Library *ExecLockBase = OpenResource((CONST_STRPTR)"execlock.resource");
    if (policy == 0 || policy > 2 || packet->dp_Port == NULL
        || (packet->dp_Port->mp_Flags & PF_ACTION) != PA_SIGNAL
        || packet->dp_Port->mp_SigTask == NULL)
        return ERROR_READ_PROTECTED;
    if (ExecLockBase)
        ObtainSystemLock(&SysBase->LibList, SPINLOCK_MODE_WRITE, LOCKF_FORBID);
    else
        Forbid();
    library = (struct Library *)FindName(&SysBase->LibList,
        (CONST_STRPTR)SECURITYNAME);
    if (library == NULL)
    {
        if (ExecLockBase)
            ReleaseSystemLock(&SysBase->LibList, LOCKF_FORBID);
        else
            Permit();
        return policy == 2 ? 0 : ERROR_READ_PROTECTED;
    }
    if (library->lib_OpenCnt >= UINT16_MAX - 1)
    {
        if (ExecLockBase)
            ReleaseSystemLock(&SysBase->LibList, LOCKF_FORBID);
        else
            Permit();
        return ERROR_READ_PROTECTED;
    }
    ++library->lib_OpenCnt;
    if (ExecLockBase)
        ReleaseSystemLock(&SysBase->LibList, LOCKF_FORBID);
    else
        Permit();
    /* The borrowed reference keeps this loaded node from being expunged;
     * the loader cannot need SYS: to open it. No list lock spans Open hooks. */
    secBase = OpenLibrary((CONST_STRPTR)SECURITYNAME, 40);
    if (ExecLockBase)
        ObtainSystemLock(&SysBase->LibList, SPINLOCK_MODE_WRITE, LOCKF_FORBID);
    else
        Forbid();
    --library->lib_OpenCnt;
    if (ExecLockBase)
        ReleaseSystemLock(&SysBase->LibList, LOCKF_FORBID);
    else
        Permit();
    if (secBase == NULL)
        return ERROR_READ_PROTECTED;
    owner = afsplus_security_task_owner(secBase, packet->dp_Port->mp_SigTask);
    CloseLibrary(secBase);
    return (owner >> 16) == secROOT_UID ? 0 : ERROR_READ_PROTECTED;
}

#endif
