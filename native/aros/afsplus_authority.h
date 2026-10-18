/* SPDX-License-Identifier: BSD-2-Clause */
#ifndef AFSPLUS_AROS_AUTHORITY_H
#define AFSPLUS_AROS_AUTHORITY_H

/* ADR-134: only the permanently loaded security 45.17+ service is admitted.
 * Its no-expunge contract makes lookup -> real Open safe without manual
 * reference-count changes. No library hook or owner query holds a list lock.
 *
 * The first admission opens the service once and keeps that open for the
 * life of the loaded handler: security's Open and Close vectors take
 * TaskOwnerSem exclusively to push and pop a context, so opening per packet
 * blocks the handler behind a root tool that holds the semaphore shared while
 * it touches this volume. Every later packet only asks for the task owner,
 * which takes the semaphore shared and joins such a holder; admission relies
 * on no exclusive holder doing I/O on an AFS+ volume, true of 45.17.
 * Handler processes that share the loaded code publish the base with a
 * compare-and-swap (Forbid where exec has one CPU and no CAS, as on m68k);
 * a loser closes its own open. The open is never returned, once per loaded
 * handler: the service is never expunged. */
static struct Library *volatile afsplus_aros_security_base;

static int afsplus_aros_publish_security(struct ExecBase *SysBase, struct Library *secBase)
{
    (void)SysBase; /* Forbid() names it on AROS */
#if defined(__aarch64__) || (defined(__GCC_ATOMIC_POINTER_LOCK_FREE) && __GCC_ATOMIC_POINTER_LOCK_FREE == 2)
    struct Library *expected = NULL;
    return __atomic_compare_exchange_n((struct Library **)&afsplus_aros_security_base,
        &expected, secBase, 0, __ATOMIC_RELEASE, __ATOMIC_RELAXED);
#else
    int published = 0;
    Forbid();
    if (afsplus_aros_security_base == NULL)
    {
        afsplus_aros_security_base = secBase;
        published = 1;
    }
    Permit();
    return published;
#endif
}

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
    if (policy == 0 || policy > 3 || packet->dp_Port == NULL
        || (packet->dp_Port->mp_Flags & PF_ACTION) != PA_SIGNAL
        || packet->dp_Port->mp_SigTask == NULL)
        return ERROR_READ_PROTECTED;
    /* ANYONE: every local task, without asking any service. */
    if (policy == 3)
        return 0;
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
    if (!afsplus_aros_publish_security(SysBase, secBase))
        CloseLibrary(secBase);
    return afsplus_aros_authorize_owner(secBase, packet);
}

#endif
