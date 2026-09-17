/* SPDX-License-Identifier: BSD-2-Clause */

/* This unit is linked into the handler next to afsplus_handler.c, which
 * already states the exec.library version the module needs; a second
 * statement of it is a duplicate symbol in the relocatable link. */
#define AROS_LIBREQ(bname, ver)

#include "afsplus_claim.h"

#include <exec/memory.h>
#include <proto/exec.h>

#include <string.h>

uint32_t afsplus_claim_name(char *name, const uint8_t *device,
    uint32_t device_length, uint64_t unit)
{
    static const char prefix[] = "AFSPLUS.";
    char digits[20];
    size_t at = sizeof(prefix) - 1;
    size_t count = 0;

    if (device == NULL || device_length == 0
        || device_length > AFSPLUS_CLAIM_DEVICE_NAME_MAX)
        return 0;
    memcpy(name, prefix, at);
    memcpy(name + at, device, device_length);
    at += device_length;
    name[at++] = '.';
    do
    {
        digits[count++] = (char)('0' + unit % 10);
        unit /= 10;
    }
    while (unit != 0);
    while (count != 0)
        name[at++] = digits[--count];
    name[at] = 0;
    return 1;
}

/* Shared by every instance of the loaded module; read and advanced under
 * Forbid(). */
static uint64_t next_generation = 1;

/* The live holder of name, or NULL. A holder whose owner is no longer a task
 * is withdrawn here: its memory was the dead task's AllocMem() and nobody
 * else will return it. Called under Forbid(); the stale claim is handed back
 * through *stale and freed by the caller after Permit(). */
static struct AfsplusArosClaim *live_holder(struct ExecBase *SysBase,
    const char *name, struct AfsplusArosClaim **stale)
{
    struct AfsplusArosClaim *holder =
        (struct AfsplusArosClaim *)FindPort((CONST_STRPTR)name);

    if (holder != NULL && !afsplus_claim_owner_alive(SysBase, holder->owner))
    {
        RemPort(&holder->port);
        *stale = holder;
        return NULL;
    }
    return holder;
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

    *claim = NULL;
    *held_generation = 0;
    if (strlen(name) >= sizeof(mine->name))
        return AFSPLUS_CLAIM_NAME_TOO_LONG;
    /* Allocated before the list is looked at: nothing may fail, or wait,
     * between the lookup and the publication. */
    mine = AllocMem(sizeof(*mine), MEMF_PUBLIC | MEMF_CLEAR);
    if (mine != NULL)
    {
        strcpy(mine->name, name);
        mine->handler_port = handler_port;
        mine->owner = owner;
        mine->port.mp_Node.ln_Type = NT_MSGPORT;
        mine->port.mp_Node.ln_Name = mine->name;
        mine->port.mp_Flags = PA_IGNORE;
        NEWLIST(&mine->port.mp_MsgList);
    }

    Forbid();
    holder = live_holder(SysBase, name, &stale);
    if (holder != NULL)
    {
        *held_generation = holder->generation;
        result = AFSPLUS_CLAIM_ALREADY_HELD;
    }
    else if (mine == NULL)
        result = AFSPLUS_CLAIM_NO_MEMORY;
    else
    {
        mine->generation = next_generation++;
        AddPort(&mine->port);
        result = AFSPLUS_CLAIM_TAKEN;
    }
    Permit();

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

    /* Withdrawn and announced in one step: a forwarder that wakes finds the
     * claim gone. */
    Forbid();
    RemPort(&claim->port);
    for (index = 0; index < AFSPLUS_CLAIM_FORWARDERS; index++)
        if (claim->forwarders[index] != NULL)
            Signal(claim->forwarders[index], AFSPLUS_CLAIM_WAKE_SIGNAL);
    Permit();
    FreeMem(claim, sizeof(*claim));
}

uint32_t afsplus_claim_enroll(struct ExecBase *sysbase, const char *name,
    uint64_t generation, struct Task *forwarder, uint32_t enroll)
{
    struct ExecBase *SysBase = sysbase;
    struct AfsplusArosClaim *holder;
    struct AfsplusArosClaim *stale = NULL;
    uint32_t served = 0;
    size_t index;

    Forbid();
    holder = live_holder(SysBase, name, &stale);
    if (holder != NULL && holder->generation == generation)
    {
        size_t free_slot = AFSPLUS_CLAIM_FORWARDERS;
        size_t own_slot = AFSPLUS_CLAIM_FORWARDERS;

        served = 1;
        for (index = 0; index < AFSPLUS_CLAIM_FORWARDERS; index++)
        {
            if (holder->forwarders[index] == forwarder)
                own_slot = index;
            else if (holder->forwarders[index] == NULL
                && free_slot == AFSPLUS_CLAIM_FORWARDERS)
                free_slot = index;
        }
        /* Enrolling is repeated at every wake-up and takes one slot once. */
        if (!enroll && own_slot != AFSPLUS_CLAIM_FORWARDERS)
            holder->forwarders[own_slot] = NULL;
        else if (enroll && own_slot == AFSPLUS_CLAIM_FORWARDERS
            && free_slot != AFSPLUS_CLAIM_FORWARDERS)
            holder->forwarders[free_slot] = forwarder;
    }
    Permit();
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
    uint32_t sent = 0;

    Forbid();
    holder = live_holder(SysBase, name, &stale);
    if (holder != NULL && holder->generation == generation)
    {
        PutMsg(holder->handler_port, message);
        sent = 1;
    }
    Permit();
    if (stale != NULL)
        FreeMem(stale, sizeof(*stale));
    return sent;
}
