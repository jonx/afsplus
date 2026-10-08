/* SPDX-License-Identifier: BSD-2-Clause */
/* Copyright (c) 2026 John Knipper */
/* Actual claim protocol, linked twice under different public symbol names.
 * This fixture supplies the shared store and protected delivery contract;
 * it does not test Exec resource publication or the kernel implementation. */
#include "afsplus_claim.h"
#include <proto/exec.h>
#include <assert.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>

void host_claim_lock(void);
void host_claim_unlock(void);
void *host_claim_alloc(size_t);
void host_claim_free(void *);
void __assert(const char *expression, const char *file, unsigned int line)
{
    printf("assertion failed: %s (%s:%u)\n", expression, file, line);
    fflush(NULL);
    abort();
}
#define DECLARE_COPY(prefix) \
uint32_t prefix##_take(struct ExecBase *, const char *, struct Task *, \
    struct MsgPort *, struct AfsplusArosClaim **, uint64_t *); \
void prefix##_release(struct ExecBase *, struct AfsplusArosClaim *); \
uint32_t prefix##_forward(struct ExecBase *, const char *, uint64_t, struct Message *)
DECLARE_COPY(copy_a);
DECLARE_COPY(copy_b);
static struct AfsplusArosClaimStore store;
static struct AfsplusArosClaim *claims[8];
static struct MsgPort ports[8];
static uint64_t generations[8];
static uint64_t last_generation;
static unsigned winner;
static _Thread_local unsigned lock_depth;
static const char key[] = "AFSPLUS.test.device.0.0.512.1.1.0.1000";

APTR AllocMem(IPTR size, ULONG flags)
{
    (void)flags;
    assert(lock_depth == 0);
    return host_claim_alloc((size_t)size);
}
void FreeMem(APTR memory, IPTR size)
{
    (void)size;
    assert(lock_depth == 0);
    host_claim_free(memory);
}
struct AfsplusArosClaimStore *afsplus_claim_store(struct ExecBase *base, uint32_t *error)
{ (void)base; (void)error; return &store; }
void afsplus_claim_lock(struct ExecBase *base, struct AfsplusArosClaimStore *s)
{
    (void)base; assert(s == &store && lock_depth == 0);
    host_claim_lock(); lock_depth++;
}
void afsplus_claim_unlock(struct ExecBase *base, struct AfsplusArosClaimStore *s)
{
    (void)base; assert(s == &store && lock_depth == 1);
    lock_depth--; host_claim_unlock();
}
void afsplus_claim_yield(struct ExecBase *base)
{ (void)base; assert(lock_depth == 0); }
uint32_t afsplus_claim_task_id(struct ExecBase *base, const struct Task *task)
{ (void)base; return (uint32_t)(uintptr_t)task; }
uint32_t afsplus_claim_deliver(struct ExecBase *base, struct Task *task,
    uint32_t id, struct MsgPort *port, struct Message *message, uint32_t signals)
{
    (void)base; assert(lock_depth == 1 && signals == 0);
    assert(id == (uintptr_t)task && id >= 1 && id <= 8);
    if (message) {
        assert(port == &ports[id - 1]);
        message->mn_Node.ln_Type = 0x42;
    }
    return AFSPLUS_CLAIM_SENT;
}
void claim_test_init(void)
{ NEWLIST(&store.claims); store.next_generation = 1; }
void claim_test_phase(unsigned index, unsigned phase)
{
    uint32_t result;
    struct Message message;
    if (phase == 0) {
        result = (index % 2 ? copy_a_take : copy_b_take)(NULL, key,
            (struct Task *)(uintptr_t)(index + 1), &ports[index],
            &claims[index], &generations[index]);
        assert(result == AFSPLUS_CLAIM_TAKEN || result == AFSPLUS_CLAIM_ALREADY_HELD);
        if (claims[index]) generations[index] = claims[index]->generation;
    } else if (phase == 1) {
        /* Barrier makes every take visible; all callers must share exactly
         * one winner, even though they execute separate module copies. */
        unsigned count = 0;
        for (unsigned i = 0; i < 8; i++) {
            count += claims[i] != NULL;
            assert(generations[i] == generations[index]);
        }
        assert(count == 1);
        if (index == 0) {
            assert(generations[0] > last_generation);
            last_generation = generations[0];
            for (unsigned i = 0; i < 8; i++) if (claims[i]) winner = i;
        }
        memset(&message, 0, sizeof(message));
        assert((index % 2 ? copy_b_forward : copy_a_forward)(NULL, key,
            generations[index], &message) == AFSPLUS_CLAIM_SENT);
        assert(message.mn_Node.ln_Type == 0x42);
    } else if (phase == 2) {
        /* Race removal against delivery through the other copy. Only a send
         * before removal or an untouched refusal after removal is legal. */
        if (index == winner)
            (index % 2 ? copy_b_release : copy_a_release)(NULL, claims[index]);
        else {
            memset(&message, 0, sizeof(message));
            result = (index % 2 ? copy_a_forward : copy_b_forward)(NULL, key,
                generations[index], &message);
            assert(result == AFSPLUS_CLAIM_SENT || result == AFSPLUS_CLAIM_GONE);
            assert(message.mn_Node.ln_Type == (result == AFSPLUS_CLAIM_SENT ? 0x42 : 0));
        }
    } else {
        assert(copy_a_forward(NULL, key, generations[index], &message) == AFSPLUS_CLAIM_GONE);
        assert(copy_b_forward(NULL, key, generations[index], &message) == AFSPLUS_CLAIM_GONE);
        claims[index] = NULL;
    }
}
void claim_test_finish(void)
{
    assert(store.claims.lh_Head->ln_Succ == NULL);
    assert(store.next_generation == 1001);
}
