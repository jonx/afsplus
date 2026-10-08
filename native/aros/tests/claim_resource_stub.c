/* SPDX-License-Identifier: BSD-2-Clause */
/* Copyright (c) 2026 John Knipper */
/* Actual OS glue with mocked Exec resource list and execlock primitives.
 * Both separately linked copies must publish the same boot-lifetime store. */
#include "afsplus_claim.h"
#include <exec/execbase.h>
#include <resources/execlock.h>
#include <aros/types/spinlock_s.h>
#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
void host_claim_lock(void);
void host_claim_unlock(void);
void host_test_barrier(void);
void *host_claim_alloc(size_t);
void host_claim_free(void *);
struct AfsplusArosClaimStore *copy_a_store(struct ExecBase *, uint32_t *);
struct AfsplusArosClaimStore *copy_b_store(struct ExecBase *, uint32_t *);
void copy_a_lock(struct ExecBase *, struct AfsplusArosClaimStore *);
void copy_b_unlock(struct ExecBase *, struct AfsplusArosClaimStore *);
static struct ExecBase base;
static struct Library locks;
static struct AfsplusArosClaimStore *stores[8];
static _Thread_local unsigned lock_depth;
static unsigned fail_allocation, fail_lock;
static unsigned allocations_count;
void __assert(const char *expression, const char *file, unsigned int line)
{
    printf("assertion failed: %s (%s:%u)\n", expression, file, line);
    fflush(NULL); abort();
}
APTR AllocMem(IPTR size, ULONG flags)
{
    (void)flags; assert(!lock_depth);
    return fail_allocation ? NULL : host_claim_alloc((size_t)size);
}
void FreeMem(APTR memory, IPTR size)
{ (void)size; assert(!lock_depth); host_claim_free(memory); }
struct Node *FindName(struct List *list, CONST_STRPTR name)
{
    struct Node *node;
    assert(lock_depth == 1);
    ForeachNode(list, node) if (!strcmp(node->ln_Name, (const char *)name)) return node;
    return NULL;
}
APTR OpenResource(CONST_STRPTR name)
{
    if (!strcmp((const char *)name, "execlock.resource")) return &locks;
    assert(!strcmp((const char *)name, AFSPLUS_CLAIM_STORE_NAME) && !lock_depth);
    host_claim_lock(); lock_depth++;
    void *result = FindName(&base.ResourceList, name);
    lock_depth--; host_claim_unlock();
    return result;
}
APTR host_alloc_lock(struct Library *library)
{
    assert(library == &locks && !lock_depth);
    if (fail_lock) return NULL;
    /* All eight candidates must exist before any publication. */
    void *lock = host_claim_alloc(8);
    host_test_barrier();
    return lock;
}
void host_free_lock(struct Library *library, APTR lock)
{ assert(library == &locks && !lock_depth); host_claim_free(lock); }
int host_obtain_system_lock(struct Library *library, struct List *list,
    ULONG mode, ULONG flags)
{
    assert(library == &locks && list == &base.ResourceList);
    assert(mode == SPINLOCK_MODE_WRITE && flags == LOCKF_DISABLE && !lock_depth);
    host_claim_lock(); lock_depth++; return 1;
}
void host_release_system_lock(struct Library *library, struct List *list, ULONG flags)
{
    assert(library == &locks && list == &base.ResourceList);
    assert(flags == LOCKF_DISABLE && lock_depth == 1);
    lock_depth--; host_claim_unlock();
}
int host_obtain_lock(struct Library *library, APTR lock, ULONG mode, ULONG flags)
{
    assert(library == &locks && lock != NULL && !lock_depth);
    assert(mode == SPINLOCK_MODE_WRITE && flags == LOCKF_DISABLE);
    host_claim_lock(); lock_depth++; return 1;
}
void host_release_lock(struct Library *library, APTR lock, ULONG flags)
{
    assert(library == &locks && lock != NULL && lock_depth == 1 && flags == LOCKF_DISABLE);
    lock_depth--; host_claim_unlock();
}
ULONG host_process_message(struct ExecBase *sysbase, struct Task *task, ULONG id,
    struct Message *message, ULONG signals)
{
    (void)sysbase; (void)task; (void)id; (void)message; (void)signals;
    abort();
}
void Reschedule(void) { assert(!lock_depth); }
void claim_test_init(void)
{
    NEWLIST(&base.ResourceList);
    base.LibNode.lib_Version = 51;
    base.LibNode.lib_Revision = 9;
    uint32_t error = 99;
    fail_allocation = 1;
    assert(copy_a_store(&base, &error) == NULL && error == AFSPLUS_CLAIM_NO_MEMORY);
    fail_allocation = 0;
    fail_lock = 1;
    assert(copy_b_store(&base, &error) == NULL && error == AFSPLUS_CLAIM_NO_MEMORY);
    fail_lock = 0;
    assert(base.ResourceList.lh_Head->ln_Succ == NULL);
}
void claim_test_phase(unsigned index, unsigned phase)
{
    if (phase == 0) {
        uint32_t error = 99;
        stores[index] = (index % 2 ? copy_a_store : copy_b_store)(&base, &error);
        assert(stores[index] != NULL);

    } else if (phase == 1) {
        for (unsigned i = 0; i < 8; i++) assert(stores[i] == stores[index]);
        assert(stores[index]->resource.ln_Name == stores[index]->resource_name);
        assert(!strcmp(stores[index]->resource_name, AFSPLUS_CLAIM_STORE_NAME));
    } else if (phase == 2) {
        copy_a_lock(&base, stores[index]);
        allocations_count++;
        copy_b_unlock(&base, stores[index]);
    }
}
void claim_test_finish(void)
{
    assert(allocations_count == 8000);
    assert(base.ResourceList.lh_Head == &stores[0]->resource);
    assert(stores[0]->resource.ln_Succ->ln_Succ == NULL);
    uint32_t error = 99;
    base.LibNode.lib_Revision = 8;
    assert(copy_a_store(&base, &error) == NULL && error == AFSPLUS_CLAIM_UNSUPPORTED);
    base.LibNode.lib_Revision = 9;
    stores[0]->abi++;
    assert(copy_b_store(&base, &error) == NULL && error == AFSPLUS_CLAIM_UNSUPPORTED);
    stores[0]->abi--;
    void *saved_lock = stores[0]->lock;
    stores[0]->lock = NULL;
    assert(copy_a_store(&base, &error) == NULL && error == AFSPLUS_CLAIM_UNSUPPORTED);
    stores[0]->lock = saved_lock;
    host_free_lock(&locks, stores[0]->lock);
    FreeMem(stores[0], sizeof(*stores[0]));
}
