/* SPDX-License-Identifier: BSD-2-Clause */

/* This unit is linked into the handler next to afsplus_handler.c, which
 * already states the exec.library version the module needs; a second
 * statement of it is a duplicate symbol in the relocatable link. */
#define AROS_LIBREQ(bname, ver)

#include "afsplus_claim.h"

#include <exec/memory.h>
#include <proto/exec.h>

#include <string.h>

static size_t append_number(char *name, size_t at, uint64_t value)
{
    char digits[20];
    size_t count = 0;

    name[at++] = '.';
    do
    {
        digits[count++] = (char)('0' + value % 10);
        value /= 10;
    }
    while (value != 0);
    while (count != 0)
        name[at++] = digits[--count];
    return at;
}

uint32_t afsplus_claim_name(char *name,
    const struct AfsplusArosClaimKey *key)
{
    static const char prefix[] = "AFSPLUS.";
    size_t at = sizeof(prefix) - 1;

    if (key->device == NULL || key->device_length == 0
        || key->device_length > AFSPLUS_CLAIM_DEVICE_NAME_MAX)
        return 0;
    memcpy(name, prefix, at);
    memcpy(name + at, key->device, key->device_length);
    at += key->device_length;
    /* Numbers, never a digest: two different media cannot share a name. */
    at = append_number(name, at, key->unit);
    at = append_number(name, at, key->flags);
    at = append_number(name, at, key->size_block);
    at = append_number(name, at, key->surfaces);
    at = append_number(name, at, key->blocks_per_track);
    at = append_number(name, at, key->low_cylinder);
    at = append_number(name, at, key->high_cylinder);
    name[at] = 0;
    return 1;
}

/* Caller holds the store lock. No pointer escapes that lock except the new
 * owner's own claim. Only the finder of a proven stale entry frees it. */
static struct AfsplusArosClaim *live_holder(struct ExecBase *SysBase,
    struct AfsplusArosClaimStore *store, const char *name,
    struct AfsplusArosClaim **stale)
{
    struct AfsplusArosClaim *holder;
    ForeachNode(&store->claims, holder)
    {
        if (strcmp(holder->name, name) != 0)
            continue;
        if (afsplus_claim_deliver(SysBase, holder->owner, holder->owner_id,
                NULL, NULL, 0) == AFSPLUS_CLAIM_GONE)
        {
            REMOVE(&holder->node);
            *stale = holder;
            return NULL;
        }
        return holder;
    }
    return NULL;
}

uint32_t afsplus_claim_take(struct ExecBase *sysbase, const char *name,
    struct Task *owner, struct MsgPort *handler_port,
    struct AfsplusArosClaim **claim, uint64_t *held_generation)
{
    struct ExecBase *SysBase = sysbase;
    struct AfsplusArosClaim *mine;
    struct AfsplusArosClaim *holder;
    struct AfsplusArosClaim *stale = NULL;
    uint32_t result;
    struct AfsplusArosClaimStore *store;

    *claim = NULL;
    *held_generation = 0;
    if (strlen(name) >= sizeof(mine->name))
        return AFSPLUS_CLAIM_NAME_TOO_LONG;
    store = afsplus_claim_store(SysBase, &result);
    if (store == NULL)
        return result;
    /* Allocated before the list is looked at: nothing may fail, or wait,
     * between the lookup and the publication. */
    mine = AllocMem(sizeof(*mine), MEMF_PUBLIC | MEMF_CLEAR);
    if (mine != NULL)
    {
        strcpy(mine->name, name);
        mine->handler_port = handler_port;
        mine->owner = owner;
        mine->store = store;
        mine->node.ln_Name = mine->name;
    }

    afsplus_claim_lock(SysBase, store);
    holder = live_holder(SysBase, store, name, &stale);
    if (holder != NULL)
    {
        *held_generation = holder->generation;
        result = AFSPLUS_CLAIM_ALREADY_HELD;
    }
    else if (store->next_generation == 0)
        result = AFSPLUS_CLAIM_EXHAUSTED;
    else if (mine == NULL)
        result = AFSPLUS_CLAIM_NO_MEMORY;
    else
    {
        mine->generation = store->next_generation++;
        mine->owner_id = afsplus_claim_task_id(SysBase, owner);
        ADDTAIL(&store->claims, &mine->node);
        result = AFSPLUS_CLAIM_TAKEN;
    }
    afsplus_claim_unlock(SysBase, store);

    if (stale != NULL)
        FreeMem(stale, sizeof(*stale));
    if (result == AFSPLUS_CLAIM_TAKEN)
        *claim = mine;
    else if (mine != NULL)
        FreeMem(mine, sizeof(*mine));
    return result;
}
void afsplus_claim_release(struct ExecBase *sysbase,
    struct AfsplusArosClaim *claim)
{
    struct ExecBase *SysBase = sysbase;

    (void)SysBase;
    if (claim == NULL)
        return;
    size_t index;

    struct AfsplusArosClaimStore *store = claim->store;
    afsplus_claim_lock(SysBase, store);
    REMOVE(&claim->node);
    afsplus_claim_unlock(SysBase, store);
    /* No lookup can reach the removed claim, and no enrolment is in flight.
     * A queue-full retry must not hold the shared claim lock. */
    for (index = 0; index < AFSPLUS_CLAIM_FORWARDERS; index++)
        if (claim->forwarders[index] != NULL)
        {
            uint32_t result;
            do
            {
                result = afsplus_claim_deliver(SysBase, claim->forwarders[index],
                    claim->forwarder_ids[index], NULL, NULL,
                    AFSPLUS_CLAIM_WAKE_SIGNAL);
                if (result == AFSPLUS_CLAIM_RETRY)
                    afsplus_claim_yield(SysBase);
            } while (result == AFSPLUS_CLAIM_RETRY);
        }
    FreeMem(claim, sizeof(*claim));
}

uint32_t afsplus_claim_enroll(struct ExecBase *sysbase, const char *name,
    uint64_t generation, struct Task *forwarder, uint32_t enroll)
{
    struct ExecBase *SysBase = sysbase;
    struct AfsplusArosClaim *holder;
    struct AfsplusArosClaim *stale = NULL;
    uint32_t served = 0;
    uint32_t error, own_id;
    struct AfsplusArosClaimStore *store = afsplus_claim_store(SysBase, &error);
    size_t index;
    if (store == NULL)
        return 0;

    afsplus_claim_lock(SysBase, store);
    holder = live_holder(SysBase, store, name, &stale);
    if (holder != NULL && holder->generation == generation)
    {
        size_t free_slot = AFSPLUS_CLAIM_FORWARDERS;
        own_id = afsplus_claim_task_id(SysBase, forwarder);
        size_t own_slot = AFSPLUS_CLAIM_FORWARDERS;

        served = 1;
        for (index = 0; index < AFSPLUS_CLAIM_FORWARDERS; index++)
        {
            if (holder->forwarders[index] == forwarder)
            {
                own_slot = index;
                if (enroll)
                    holder->forwarder_ids[index] = own_id;
            }
            else if (holder->forwarders[index] == NULL
                && free_slot == AFSPLUS_CLAIM_FORWARDERS)
                free_slot = index;
        }
        /* Enrolling is repeated at every wake-up and takes one slot once. */
        if (!enroll && own_slot != AFSPLUS_CLAIM_FORWARDERS)
            holder->forwarders[own_slot] = NULL;
        else if (enroll && own_slot == AFSPLUS_CLAIM_FORWARDERS
            && free_slot != AFSPLUS_CLAIM_FORWARDERS)
        {
            holder->forwarders[free_slot] = forwarder;
            holder->forwarder_ids[free_slot] =
                own_id;
        }
    }
    afsplus_claim_unlock(SysBase, store);
    if (stale != NULL)
        FreeMem(stale, sizeof(*stale));
    return served;
}

uint32_t afsplus_claim_forward(struct ExecBase *sysbase, const char *name,
    uint64_t generation, struct Message *message)
{
    struct ExecBase *SysBase = sysbase;
    struct AfsplusArosClaim *holder;
    struct AfsplusArosClaim *stale = NULL;
    uint32_t sent = 0, error;
    struct AfsplusArosClaimStore *store = afsplus_claim_store(SysBase, &error);
    if (store == NULL)
        return AFSPLUS_CLAIM_SEND_ERROR;

    afsplus_claim_lock(SysBase, store);
    holder = live_holder(SysBase, store, name, &stale);
    if (holder != NULL && holder->generation == generation)
    {
        sent = afsplus_claim_deliver(SysBase, holder->owner, holder->owner_id,
            holder->handler_port, message, 0);
    }
    afsplus_claim_unlock(SysBase, store);
    if (stale != NULL)
        FreeMem(stale, sizeof(*stale));
    return sent;
}
