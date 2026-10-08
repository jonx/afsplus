/* SPDX-License-Identifier: BSD-2-Clause */
/* Copyright (c) 2026 John Knipper */
#define AROS_LIBREQ(bname, ver)
#define __EXECLOCK_NOLIBBASE__
#include "afsplus_claim.h"
#include <exec/execbase.h>
#include <exec/memory.h>
#include <dos/dosextens.h>
#include <proto/exec.h>
#include <string.h>
#if defined(__AROSEXEC_SMP__)
#include <exec/processmessage.h>
#include <aros/types/spinlock_s.h>
#include <resources/execlock.h>
#include <proto/execlock.h>
#endif

struct AfsplusArosClaimStore *afsplus_claim_store(struct ExecBase *SysBase,
    uint32_t *error)
{
    struct AfsplusArosClaimStore *store, *candidate;
#if defined(__AROSEXEC_SMP__)
    struct Library *ExecLockBase;
    *error = AFSPLUS_CLAIM_UNSUPPORTED;
    /* Do not call vector 180 on an Exec that does not implement it. */
    if (SysBase->LibNode.lib_Version < 51 ||
        (SysBase->LibNode.lib_Version == 51 && SysBase->LibNode.lib_Revision < 9))
        return NULL;
    ExecLockBase = OpenResource((CONST_STRPTR)"execlock.resource");
    if (ExecLockBase == NULL)
        return NULL;
#endif
    store = OpenResource((CONST_STRPTR)AFSPLUS_CLAIM_STORE_NAME);
    if (store == NULL)
    {
        *error = AFSPLUS_CLAIM_NO_MEMORY;
        candidate = AllocMem(sizeof(*candidate), MEMF_PUBLIC | MEMF_CLEAR);
        if (candidate != NULL)
        {
            strcpy(candidate->resource_name, AFSPLUS_CLAIM_STORE_NAME);
            candidate->resource.ln_Name = candidate->resource_name;
            candidate->resource.ln_Type = NT_RESOURCE;
            candidate->abi = AFSPLUS_CLAIM_STORE_ABI;
            candidate->bytes = sizeof(*candidate);
            candidate->next_generation = 1;
            NEWLIST(&candidate->claims);
#if defined(__AROSEXEC_SMP__)
            candidate->lock_base = ExecLockBase;
            candidate->lock = AllocLock();
            if (candidate->lock == NULL)
            {
                FreeMem(candidate, sizeof(*candidate));
                candidate = NULL;
            }
#endif
        }
        /* Even allocation failure must observe a winner published meanwhile.
         * Use raw list operations under this lock, not recursively locking
         * OpenResource/AddResource calls. The resource is never withdrawn. */
#if defined(__AROSEXEC_SMP__)
        ObtainSystemLock(&SysBase->ResourceList, SPINLOCK_MODE_WRITE, LOCKF_DISABLE);
#else
        Forbid();
#endif
        store = (struct AfsplusArosClaimStore *)FindName(&SysBase->ResourceList,
            (CONST_STRPTR)AFSPLUS_CLAIM_STORE_NAME);
        if (store == NULL && candidate != NULL)
        {
            store = candidate;
            ADDTAIL(&SysBase->ResourceList, &candidate->resource);
            candidate = NULL;
        }
#if defined(__AROSEXEC_SMP__)
        ReleaseSystemLock(&SysBase->ResourceList, LOCKF_DISABLE);
#else
        Permit();
#endif
        if (candidate != NULL)
        {
#if defined(__AROSEXEC_SMP__)
            FreeLock(candidate->lock);
#endif
            FreeMem(candidate, sizeof(*candidate));
        }
    }
    if (store != NULL && (store->abi != AFSPLUS_CLAIM_STORE_ABI ||
            store->bytes != sizeof(*store)))
    {
        *error = AFSPLUS_CLAIM_UNSUPPORTED;
        return NULL;
    }
#if defined(__AROSEXEC_SMP__)
    if (store != NULL && (store->lock == NULL || store->lock_base != ExecLockBase))
    {
        *error = AFSPLUS_CLAIM_UNSUPPORTED;
        return NULL;
    }
#endif
    return store;
}

void afsplus_claim_lock(struct ExecBase *SysBase, struct AfsplusArosClaimStore *store)
{
#if defined(__AROSEXEC_SMP__)
    struct Library *ExecLockBase = store->lock_base;
    (void)SysBase;
    ObtainLock(store->lock, SPINLOCK_MODE_WRITE, LOCKF_DISABLE);
#else
    (void)store;
    Forbid();
#endif
}

void afsplus_claim_unlock(struct ExecBase *SysBase, struct AfsplusArosClaimStore *store)
{
#if defined(__AROSEXEC_SMP__)
    struct Library *ExecLockBase = store->lock_base;
    (void)SysBase;
    ReleaseLock(store->lock, LOCKF_DISABLE);
#else
    (void)store;
    Permit();
#endif
}

uint32_t afsplus_claim_task_id(struct ExecBase *SysBase, const struct Task *task)
{
    (void)SysBase;
    if (task == NULL || !(task->tc_Flags & TF_ETASK) ||
        task->tc_UnionETask.tc_ETask == NULL)
        return 0;
    return task->tc_UnionETask.tc_ETask->et_UniqueID;
}

#if !defined(__AROSEXEC_SMP__)
static uint32_t task_alive(struct ExecBase *SysBase,
    const struct Task *task, uint32_t id)
{
    const struct Node *node;
    uint32_t listed = task == SysBase->ThisTask;
    if (task == NULL)
        return 0;
    for (node = SysBase->TaskReady.lh_Head; !listed && node->ln_Succ != NULL;
        node = node->ln_Succ)
        listed = (const struct Task *)node == task;
    for (node = SysBase->TaskWait.lh_Head; !listed && node->ln_Succ != NULL;
        node = node->ln_Succ)
        listed = (const struct Task *)node == task;
    return listed && task->tc_Node.ln_Type == NT_PROCESS &&
        afsplus_claim_task_id(SysBase, task) == id;
}
#endif

uint32_t afsplus_claim_deliver(struct ExecBase *SysBase, struct Task *task,
    uint32_t id, struct MsgPort *port, struct Message *message, uint32_t signals)
{
#if defined(__AROSEXEC_SMP__)
    ULONG result;
    /* The handler always uses its process's embedded pr_MsgPort. */
    (void)port;
    result = TryProcessMessage(task, id, message, signals);
    if (result == TPM_OK)
        return AFSPLUS_CLAIM_SENT;
    if (result == TPM_DEAD)
        return AFSPLUS_CLAIM_GONE;
    if (result == TPM_BUSY)
        return AFSPLUS_CLAIM_RETRY;
    return AFSPLUS_CLAIM_SEND_ERROR;
#else
    uint32_t result = AFSPLUS_CLAIM_GONE;
    Forbid();
    if (task_alive(SysBase, task, id))
    {
        if (message != NULL)
            PutMsg(port, message);
        if (signals != 0)
            Signal(task, signals);
        result = AFSPLUS_CLAIM_SENT;
    }
    Permit();
    return result;
#endif
}

void afsplus_claim_yield(struct ExecBase *SysBase)
{
    (void)SysBase;
    Reschedule();
}
