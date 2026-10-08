/* SPDX-License-Identifier: BSD-2-Clause */

/* Host matrix of the one-instance claim over a shared store and faked Exec:
 * who gets the unit, what a later instance does, and what a forwarder does
 * when the claimed instance has gone or has been restarted. */

#include "afsplus_claim.h"

#include <proto/exec.h>

#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

void __assert(const char *expression, const char *file, unsigned int line)
{
    printf("assertion failed: %s (%s:%u)\n", expression, file, line);
    fflush(NULL);
    abort();
}

/* The OS adapter supplies one boot-lifetime store across module copies. */
static struct AfsplusArosClaimStore store;
static int initialized;
static int forbid_depth;
static int allocations;
static int fail_allocation;
static struct MsgPort *last_target;
static struct Message *last_message;
static int puts_outside_forbid;
static int busy_deliveries;
static int yield_count;

void Forbid(void) { forbid_depth++; }
void Permit(void) { assert(forbid_depth > 0); forbid_depth--; }
struct AfsplusArosClaimStore *afsplus_claim_store(struct ExecBase *sysbase,
    uint32_t *error)
{
    (void)sysbase; (void)error;
    if (!initialized)
    {
        NEWLIST(&store.claims);
        store.next_generation = 1;
        initialized = 1;
    }
    return &store;
}
void afsplus_claim_lock(struct ExecBase *sysbase, struct AfsplusArosClaimStore *s)
{ (void)sysbase; assert(s == &store); Forbid(); }
void afsplus_claim_unlock(struct ExecBase *sysbase, struct AfsplusArosClaimStore *s)
{ (void)sysbase; assert(s == &store); Permit(); }
void afsplus_claim_yield(struct ExecBase *sysbase)
{ (void)sysbase; assert(forbid_depth == 0); yield_count++; }

APTR AllocMem(IPTR size, ULONG requirements)
{
    (void)requirements;
    /* Memory is never requested with the list locked. */
    assert(forbid_depth == 0);
    if (fail_allocation)
        return NULL;
    allocations++;
    return calloc(1, size);
}

void FreeMem(APTR memory, IPTR size)
{
    (void)size;
    assert(forbid_depth == 0);
    allocations--;
    free(memory);
}

/* The faked system's tasks: an address is alive unless it is dead_task, and
 * carries the unique ID the table says, so an address can be reissued. */
static struct Task *dead_task;
static struct Task *retiring_task;
static struct Task *reissued_task;
static uint32_t reissued_id;

uint32_t afsplus_claim_task_id(struct ExecBase *sysbase,
    const struct Task *task)
{
    (void)sysbase;
    assert(forbid_depth > 0);
    if (task == reissued_task)
        return reissued_id;
    return (uint32_t)((uintptr_t)task >> 4);
}

uint32_t afsplus_claim_task_alive(struct ExecBase *sysbase,
    const struct Task *task, uint32_t task_id)
{
    assert(forbid_depth > 0);
    return task != NULL && task != dead_task
        && afsplus_claim_task_id(sysbase, task) == task_id;
}

static struct Task *signalled[4];
static size_t signalled_count;

void Signal(struct Task *task, ULONG signals)
{
    assert(forbid_depth > 0);
    assert(signals == AFSPLUS_CLAIM_WAKE_SIGNAL);
    assert(signalled_count < 4);
    signalled[signalled_count++] = task;
}

void PutMsg(struct MsgPort *port, struct Message *message)
{
    /* The lookup and the send are one step: the target cannot vanish
     * between them. */
    if (forbid_depth == 0)
        puts_outside_forbid++;
    last_target = port;
    last_message = message;
}

uint32_t afsplus_claim_deliver(struct ExecBase *sysbase, struct Task *task,
    uint32_t id, struct MsgPort *port, struct Message *message, uint32_t signals)
{
    uint32_t result = AFSPLUS_CLAIM_GONE;
    Forbid();
    if (task == retiring_task)
        result = AFSPLUS_CLAIM_RETRY;
    else if (afsplus_claim_task_alive(sysbase, task, id))
    {
        result = AFSPLUS_CLAIM_SENT;
        if ((message != NULL || signals != 0) && busy_deliveries > 0)
        {
            busy_deliveries--;
            result = AFSPLUS_CLAIM_RETRY;
        }
        else
        {
            if (message != NULL) PutMsg(port, message);
            if (signals != 0) Signal(task, signals);
        }
    }
    Permit();
    return result;
}

int main(void)
{
    struct MsgPort first_port;
    struct MsgPort second_port;
    struct MsgPort restarted_port;
    struct AfsplusArosClaim *first = NULL;
    struct AfsplusArosClaim *second = NULL;
    struct AfsplusArosClaim *other_unit = NULL;
    struct Message message;
    struct Task *first_task = (struct Task *)0x1000;
    struct Task *second_task = (struct Task *)0x2000;
    uint64_t seen = 0;
    uint64_t unused = 0;
    struct AfsplusArosClaimKey key;
    struct AfsplusArosClaimKey other_key;
    struct AfsplusArosClaimKey big;
    char name[AFSPLUS_CLAIM_NAME_BYTES];
    char other[AFSPLUS_CLAIM_NAME_BYTES];
    uint8_t long_device[AFSPLUS_CLAIM_DEVICE_NAME_MAX + 1];

    /* The key is the medium: device, unit, flags and the partition's place
     * on the unit, as numbers. Nothing of the DOS node is in it. */
    memset(&key, 0, sizeof(key));
    key.device = (const uint8_t *)"ata.device";
    key.device_length = 10;
    key.unit = 0;
    key.flags = 0;
    key.size_block = 1024;
    key.surfaces = 16;
    key.blocks_per_track = 63;
    key.low_cylinder = 2;
    key.high_cylinder = 4095;
    assert(afsplus_claim_name(name, &key));
    assert(strcmp(name, "AFSPLUS.ata.device.0.0.1024.16.63.2.4095") == 0);

    /* Two partitions of one disk share device and unit and nothing else:
     * two names, two first instances, no forwarding between them. */
    {
        struct AfsplusArosClaimKey second_partition = key;
        struct AfsplusArosClaim *dh0 = NULL;
        struct AfsplusArosClaim *dh1 = NULL;
        uint64_t generation = 0;

        second_partition.low_cylinder = 4096;
        second_partition.high_cylinder = 8191;
        assert(afsplus_claim_name(other, &second_partition));
        assert(strcmp(name, other) != 0);
        assert(afsplus_claim_take(NULL, name, (struct Task *)0x5000,
            &first_port, &dh0, &generation) == AFSPLUS_CLAIM_TAKEN);
        assert(afsplus_claim_take(NULL, other, (struct Task *)0x6000,
            &second_port, &dh1, &generation) == AFSPLUS_CLAIM_TAKEN);
        assert(dh0 != NULL && dh1 != NULL && dh0 != dh1);
        last_target = NULL;
        assert(afsplus_claim_forward(NULL, other, dh1->generation, &message)
            == 1);
        assert(last_target == &second_port);
        afsplus_claim_release(NULL, dh0);
        afsplus_claim_release(NULL, dh1);
        /* Every number of the key separates. */
        second_partition = key;
        second_partition.flags = 1;
        assert(afsplus_claim_name(other, &second_partition));
        assert(strcmp(name, other) != 0);
        second_partition = key;
        second_partition.size_block = 512;
        assert(afsplus_claim_name(other, &second_partition));
        assert(strcmp(name, other) != 0);
    }
    /* The largest key fits its buffer; a device name too long or missing has
     * no name. */
    memset(long_device, 'd', sizeof(long_device));
    big = key;
    big.device = long_device;
    big.device_length = AFSPLUS_CLAIM_DEVICE_NAME_MAX;
    big.unit = big.flags = big.size_block = big.surfaces = UINT64_MAX;
    big.blocks_per_track = big.low_cylinder = big.high_cylinder = UINT64_MAX;
    assert(afsplus_claim_name(other, &big));
    assert(strlen(other) == AFSPLUS_CLAIM_NAME_BYTES - 1);
    big.device_length = sizeof(long_device);
    assert(!afsplus_claim_name(other, &big));
    big.device = NULL;
    assert(!afsplus_claim_name(other, &big));

    /* The first instance gets the unit; a second one does not, publishes
     * nothing, keeps nothing allocated and learns whom it serves. */
    assert(afsplus_claim_take(NULL, name, first_task, &first_port, &first,
        &seen) == AFSPLUS_CLAIM_TAKEN);
    assert(first != NULL && allocations == 1 && seen == 0);
    assert(afsplus_claim_take(NULL, name, second_task, &second_port, &second,
        &seen) == AFSPLUS_CLAIM_ALREADY_HELD);
    assert(second == NULL && allocations == 1);
    assert(seen == first->generation && seen != 0);
    /* Retirement is not death: keep the reservation until cleanup removes
     * the identity. A retry transfers no packet and admits no new writer. */
    retiring_task = first_task;
    assert(afsplus_claim_take(NULL, name, second_task, &second_port, &second,
        &seen) == AFSPLUS_CLAIM_ALREADY_HELD);
    assert(second == NULL && allocations == 1 && seen == first->generation);
    last_target = NULL;
    memset(&message, 0x5a, sizeof(message));
    {
        struct Message before = message;
        assert(afsplus_claim_forward(NULL, name, seen, &message)
            == AFSPLUS_CLAIM_RETRY);
        assert(last_target == NULL && memcmp(&message, &before, sizeof(message)) == 0);
    }
    retiring_task = NULL;
    /* Another unit of the same device is another medium. */
    other_key = key;
    other_key.unit = 1;
    assert(afsplus_claim_name(other, &other_key));
    assert(afsplus_claim_take(NULL, other, second_task, &second_port,
        &other_unit, &unused) == AFSPLUS_CLAIM_TAKEN);
    afsplus_claim_release(NULL, other_unit);
    assert(allocations == 1);

    /* Without memory an instance still learns that the unit is served: it
     * must forward, which needs none. Failing its startup would make
     * dos.library clear the live instance's dn_Task. */
    fail_allocation = 1;
    assert(afsplus_claim_take(NULL, name, second_task, &second_port, &second,
        &unused) == AFSPLUS_CLAIM_ALREADY_HELD);
    assert(unused == seen);
    /* Only an unserved unit without memory is a failed startup. */
    assert(afsplus_claim_take(NULL, other, second_task, &second_port,
        &other_unit, &unused) == AFSPLUS_CLAIM_NO_MEMORY);
    fail_allocation = 0;
    assert(other_unit == NULL && allocations == 1);

    /* Queue pressure transfers nothing and permits retry with this generation. */
    busy_deliveries = 1;
    last_target = NULL;
    assert(afsplus_claim_forward(NULL, name, seen, &message) == AFSPLUS_CLAIM_RETRY);
    assert(last_target == NULL && allocations == 1);
    /* A forwarder sends to the instance it serves. */
    assert(afsplus_claim_forward(NULL, name, seen, &message) == 1);
    assert(last_target == &first_port && last_message == &message);

    /* Forwarders enroll with the instance they serve and are woken when it
     * goes, so none stays behind idle; one that left is not woken, and one
     * that names another generation is not enrolled at all. */
    {
        struct Task *forwarder_a = (struct Task *)0x3000;
        struct Task *forwarder_b = (struct Task *)0x4000;

        assert(afsplus_claim_enroll(NULL, name, seen, forwarder_a, 1) == 1);
        assert(afsplus_claim_enroll(NULL, name, seen, forwarder_b, 1) == 1);
        /* Enrolling again, as every wake-up does, takes no second slot. */
        assert(afsplus_claim_enroll(NULL, name, seen, forwarder_a, 1) == 1);
        assert(afsplus_claim_enroll(NULL, name, seen + 1, second_task, 1)
            == 0);
        assert(afsplus_claim_enroll(NULL, name, seen, forwarder_b, 0) == 1);
        /* A forwarder that was killed while enrolled: its address now
         * belongs to another task, which must not be signalled. */
        assert(afsplus_claim_enroll(NULL, name, seen, forwarder_b, 1) == 1);
        reissued_task = forwarder_b;
        reissued_id = 0x7777;
        signalled_count = 0;
    }

    /* The served instance has gone: the message is not sent anywhere and is
     * still the forwarder's to answer. No stale port is ever used. */
    busy_deliveries = 2;
    afsplus_claim_release(NULL, first);
    assert(yield_count == 2);
    assert(allocations == 0);
    assert(signalled_count == 1 && signalled[0] == (struct Task *)0x3000);
    assert(afsplus_claim_enroll(NULL, name, seen, (struct Task *)0x3000, 1)
        == 0);
    last_target = NULL;
    assert(afsplus_claim_forward(NULL, name, seen, &message) == 0);
    assert(last_target == NULL);

    /* A restarted instance claims the same unit under the same name. The old
     * forwarder does not hand it packets that name the old instance's locks;
     * a forwarder of the new instance does reach it. */
    assert(afsplus_claim_take(NULL, name, first_task, &restarted_port, &first,
        &unused) == AFSPLUS_CLAIM_TAKEN);
    assert(first->generation != seen);
    assert(afsplus_claim_forward(NULL, name, seen, &message) == 0);
    assert(last_target == NULL);
    assert(afsplus_claim_forward(NULL, name, first->generation, &message)
        == 1);
    assert(last_target == &restarted_port);

    /* The owner crashed: its claim is still published and its port is dead
     * memory. A forwarder sends nothing there and withdraws the claim. */
    dead_task = first_task;
    seen = first->generation;
    last_target = NULL;
    assert(afsplus_claim_forward(NULL, name, seen, &message) == 0);
    assert(last_target == NULL && allocations == 0);
    /* And a new start after such a crash becomes the first instance instead
     * of a forwarder to nowhere. */
    dead_task = NULL;
    assert(afsplus_claim_take(NULL, name, first_task, &first_port, &first,
        &unused) == AFSPLUS_CLAIM_TAKEN);
    dead_task = first_task;
    assert(afsplus_claim_take(NULL, name, second_task, &second_port, &second,
        &unused) == AFSPLUS_CLAIM_TAKEN);
    assert(second != NULL && second->handler_port == &second_port);
    assert(allocations == 1);
    dead_task = NULL;
    afsplus_claim_release(NULL, second);
    afsplus_claim_release(NULL, NULL);

    /* The owner died and exec gave its address to another process: the
     * address is on the task lists again, the unique ID is not the claim's.
     * Nothing is sent to it, and the next start is a first instance. */
    reissued_task = NULL;
    assert(afsplus_claim_take(NULL, name, first_task, &first_port, &first,
        &unused) == AFSPLUS_CLAIM_TAKEN);
    seen = first->generation;
    reissued_task = first_task;
    reissued_id = 0x4242;
    last_target = NULL;
    assert(afsplus_claim_forward(NULL, name, seen, &message) == 0);
    assert(last_target == NULL && allocations == 0);
    assert(afsplus_claim_take(NULL, name, second_task, &second_port, &second,
        &unused) == AFSPLUS_CLAIM_TAKEN);
    afsplus_claim_release(NULL, second);
    reissued_task = NULL;

    /* The generation counter never silently wraps and aliases an old forwarder. */
    store.next_generation = UINT64_MAX;
    assert(afsplus_claim_take(NULL, name, first_task, &first_port, &first,
        &seen) == AFSPLUS_CLAIM_TAKEN);
    assert(first->generation == UINT64_MAX);
    assert(afsplus_claim_take(NULL, name, second_task, &second_port, &second,
        &seen) == AFSPLUS_CLAIM_ALREADY_HELD);
    afsplus_claim_release(NULL, first);
    assert(afsplus_claim_take(NULL, name, first_task, &first_port, &first,
        &seen) == AFSPLUS_CLAIM_EXHAUSTED);
    assert(first == NULL);
    assert(puts_outside_forbid == 0 && forbid_depth == 0);
    assert(allocations == 0);
    puts("afsplus claim stub: PASS");
    return 0;
}
