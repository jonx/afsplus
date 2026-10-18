/* SPDX-License-Identifier: BSD-2-Clause */
#ifndef AFSPLUS_AROS_AUTHORITY_H
#define AFSPLUS_AROS_AUTHORITY_H

/* ADR-134: only the permanently loaded security 45.17+ service is admitted.
 * Its no-expunge contract makes lookup -> real Open safe without manual
 * reference-count changes. No library hook or owner query holds a list lock. */
static int32_t afsplus_aros_authorize_policy(struct ExecBase *SysBase,
    const struct DosPacket *packet, uint32_t policy)
{
    struct Library *secBase;
    struct Library *library;
    ULONG owner;
    int supported;
    struct Library *ExecLockBase;
    if (policy == 0 || policy > 2 || packet->dp_Port == NULL
        || (packet->dp_Port->mp_Flags & PF_ACTION) != PA_SIGNAL
        || packet->dp_Port->mp_SigTask == NULL)
        return ERROR_READ_PROTECTED;
    ExecLockBase = OpenResource((CONST_STRPTR)"execlock.resource");
    if (ExecLockBase)
        ObtainSystemLock(&SysBase->LibList, SPINLOCK_MODE_READ, LOCKF_FORBID);
    else
        Forbid();
    library = (struct Library *)FindName(&SysBase->LibList,
        (CONST_STRPTR)SECURITYNAME);
    supported = library != NULL
        && library->lib_Version == AFSPLUS_SECURITY_VERSION
        && library->lib_Revision >= AFSPLUS_SECURITY_REVISION;
    if (ExecLockBase)
        ReleaseSystemLock(&SysBase->LibList, LOCKF_FORBID);
    else
        Permit();
    if (library == NULL)
        return policy == 2 ? 0 : ERROR_READ_PROTECTED;
    if (!supported)
        return ERROR_READ_PROTECTED;
    secBase = OpenLibrary((CONST_STRPTR)SECURITYNAME, AFSPLUS_SECURITY_VERSION);
    if (secBase == NULL)
        return ERROR_READ_PROTECTED;
    if (secBase != library)
    {
        CloseLibrary(secBase);
        return ERROR_READ_PROTECTED;
    }
    owner = afsplus_security_task_owner(secBase, packet->dp_Port->mp_SigTask);
    CloseLibrary(secBase);
    return (owner >> 16) == AFSPLUS_SECURITY_ROOT_UID ? 0 : ERROR_READ_PROTECTED;
}

#endif
