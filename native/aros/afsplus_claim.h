/* SPDX-License-Identifier: BSD-2-Clause */

#ifndef AFSPLUS_AROS_CLAIM_H
#define AFSPLUS_AROS_CLAIM_H

/* One handler per exact medium key. A boot-lifetime resource owns the claim
 * list, exclusion primitive and generation counter across separately loaded
 * handler modules. UP uses Forbid; SMP uses execlock.resource and the kernel's
 * checked process-message operation. Only proven-dead owners are reclaimed:
 * an unpublished/retiring owner or notification backpressure is not death.
 * Forwarders never take ownership and never forward across generations.
 * Allocation, deallocation and retry/yield happen outside the claim lock. */
#include <exec/ports.h>
#include <stdint.h>

struct ExecBase;

/* "AFSPLUS." + device name, then "." and a decimal number for each of the
 * seven numbers of the key. */
#define AFSPLUS_CLAIM_DEVICE_NAME_MAX 96
#define AFSPLUS_CLAIM_KEY_NUMBERS 7
#define AFSPLUS_CLAIM_NAME_BYTES \
    (8 + AFSPLUS_CLAIM_DEVICE_NAME_MAX + AFSPLUS_CLAIM_KEY_NUMBERS * 21 + 1)
/* Forwarders an instance can wake. One beyond them still forwards and still
 * ends on the first packet it cannot forward; it is only not woken, so it
 * stays if no packet ever reaches it. Ending it at once instead would lose
 * the packet its one caller is about to send. */
#define AFSPLUS_CLAIM_FORWARDERS 32

/* What places a filesystem on a device: FileSysStartupMsg and the DosEnvec
 * fields that bound the partition. */
struct AfsplusArosClaimKey {
    const uint8_t *device;
    uint32_t device_length;
    uint64_t unit;
    uint64_t flags;
    uint64_t size_block;
    uint64_t surfaces;
    uint64_t blocks_per_track;
    uint64_t low_cylinder;
    uint64_t high_cylinder;
};
/* The signal a released claim sends its forwarders: SIGBREAKF_CTRL_C. */
#define AFSPLUS_CLAIM_WAKE_SIGNAL (UINT32_C(1) << 12)

struct Task;

struct Library;
struct AfsplusArosClaimStore {
    struct Node resource;
    uint32_t abi;
    uint32_t bytes;
    struct List claims;
    uint64_t next_generation;
    struct Library *lock_base;
    void *lock;
    char resource_name[32];
};
#define AFSPLUS_CLAIM_STORE_NAME "afsplus.claims"
#define AFSPLUS_CLAIM_STORE_ABI 1

struct AfsplusArosClaim {
    struct Node node;
    struct AfsplusArosClaimStore *store;
    struct MsgPort *handler_port;
    struct Task *owner;
    uint32_t owner_id;
    /* Distinguishes this instance from every earlier and later holder of the
     * same name, also when the owner's task address is reused. */
    uint64_t generation;
    /* Forwarders that serve this instance, woken when it releases the claim
     * so that none outlives it idle. A forwarder that finds no free slot
     * still ends with the next packet it cannot forward. */
    struct Task *forwarders[AFSPLUS_CLAIM_FORWARDERS];
    uint32_t forwarder_ids[AFSPLUS_CLAIM_FORWARDERS];
    char name[AFSPLUS_CLAIM_NAME_BYTES];
};

/* OS adapter. The store persists until reboot and owns no module pointers.
 * task_id is used only for the calling owner/forwarder itself. deliver checks
 * recorded identities without dereferencing a stale pointer. */
uint32_t afsplus_claim_task_id(struct ExecBase *, const struct Task *);
struct AfsplusArosClaimStore *afsplus_claim_store(struct ExecBase *, uint32_t *error);
void afsplus_claim_lock(struct ExecBase *, struct AfsplusArosClaimStore *);
void afsplus_claim_unlock(struct ExecBase *, struct AfsplusArosClaimStore *);
#define AFSPLUS_CLAIM_GONE 0
#define AFSPLUS_CLAIM_SENT 1
#define AFSPLUS_CLAIM_RETRY 2
#define AFSPLUS_CLAIM_SEND_ERROR 3
uint32_t afsplus_claim_deliver(struct ExecBase *, struct Task *, uint32_t id,
    struct MsgPort *, struct Message *, uint32_t signals);
void afsplus_claim_yield(struct ExecBase *);

#define AFSPLUS_CLAIM_TAKEN 0
#define AFSPLUS_CLAIM_ALREADY_HELD 1
#define AFSPLUS_CLAIM_NO_MEMORY 2
#define AFSPLUS_CLAIM_NAME_TOO_LONG 3
#define AFSPLUS_CLAIM_UNSUPPORTED 4
#define AFSPLUS_CLAIM_EXHAUSTED 5

/* Builds the claim name; 0 when the device name is empty or does not fit. */
uint32_t afsplus_claim_name(char *name,
    const struct AfsplusArosClaimKey *key);

/* Publishes the claim for name on behalf of owner and its handler_port. On
 * AFSPLUS_CLAIM_TAKEN *claim is the published claim; otherwise it is NULL and
 * nothing was published. AFSPLUS_CLAIM_ALREADY_HELD stores the holder's
 * generation and needs no memory: an instance that cannot allocate still
 * learns that it must forward, never that it should fail its startup while
 * another instance is live. */
uint32_t afsplus_claim_take(struct ExecBase *sysbase, const char *name,
    struct Task *owner, struct MsgPort *handler_port,
    struct AfsplusArosClaim **claim, uint64_t *held_generation);

/* Withdraws and frees a claim; NULL is allowed. After it returns no forwarder
 * queues another message on the handler port. */
void afsplus_claim_release(struct ExecBase *sysbase,
    struct AfsplusArosClaim *claim);

/* Registers, or with enroll 0 removes, a forwarder task with the live
 * instance that holds name with this generation. Returns 1 while that
 * instance is there, 0 when it has gone: the forwarder then ends. */
uint32_t afsplus_claim_enroll(struct ExecBase *sysbase, const char *name,
    uint64_t generation, struct Task *forwarder, uint32_t enroll);

/* SENT transfers message ownership; GONE/SEND_ERROR leaves it for the caller
 * to answer. RETRY leaves it byte-for-byte unchanged: yield outside the claim
 * lock and call again, rechecking the generation on every attempt. */
uint32_t afsplus_claim_forward(struct ExecBase *sysbase, const char *name,
    uint64_t generation, struct Message *message);

#endif
