/* SPDX-License-Identifier: BSD-2-Clause */
#ifndef AFSPLUS_AROS_AUTHORITY_H
#define AFSPLUS_AROS_AUTHORITY_H

/* ADR-134: only the permanently loaded security 45.17+ service is admitted.
 * Its no-expunge contract makes lookup -> real Open safe without manual
 * reference-count changes. No library hook or owner query holds a list lock.
 *
 * The first admission opens the service once and keeps that open for the
 * life of the loaded handler: security's Open and Close vectors push and pop
 * a context under TaskOwnerSem, so opening per packet could block the handler
 * behind a root tool that holds the semaphore while it touches this volume.
 * Every later packet only asks for the task owner. Handler processes that
 * share the loaded code race for the slot under Forbid; a loser closes its
 * own open. The open is never returned: the service is never expunged. */
static struct Library *volatile afsplus_aros_security_base;

static int32_t afsplus_aros_authorize_owner(struct Library *secBase,
    const struct DosPacket *packet)
{
    ULONG owner = afsplus_security_task_owner(secBase, packet->dp_Port->mp_SigTask);
    return (owner >> 16) == AFSPLUS_SECURITY_ROOT_UID ? 0 : ERROR_READ_PROTECTED;
}

static int32_t afsplus_aros_authorize_policy(struct ExecBase *SysBase,
    const struct DosPacket *packet, uint32_t policy)
{
    struct Library *secBase;
    struct Library *library;
    int supported;
    struct Library *ExecLockBase;
    if (policy == 0 || policy > 2 || packet->dp_Port == NULL
        || (packet->dp_Port->mp_Flags & PF_ACTION) != PA_SIGNAL
        || packet->dp_Port->mp_SigTask == NULL)
        return ERROR_READ_PROTECTED;
    secBase = afsplus_aros_security_base;
    if (secBase != NULL)
        return afsplus_aros_authorize_owner(secBase, packet);
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
    Forbid();
    if (afsplus_aros_security_base == NULL)
    {
        afsplus_aros_security_base = secBase;
        library = NULL;
    }
    Permit();
    if (library != NULL)
        CloseLibrary(secBase);
    return afsplus_aros_authorize_owner(afsplus_aros_security_base, packet);
}

#endif
