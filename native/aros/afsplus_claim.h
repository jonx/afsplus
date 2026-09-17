/* SPDX-License-Identifier: BSD-2-Clause */

#ifndef AFSPLUS_AROS_CLAIM_H
#define AFSPLUS_AROS_CLAIM_H

/*
 * One handler instance per backing device unit.
 *
 * dos.library starts a handler from RunHandler() when dn_Task is NULL, without
 * serialising its callers: Mount defers the start to the first access, and two
 * tasks that make their first access together each get a handler process. Two
 * instances on one volume are two writers on one image. The instance that
 * comes first therefore publishes a claim, a public port named after the
 * device and unit it is about to open, before it opens them. The key is the
 * medium, not the DOS node: a second DOS name for the same unit, or a node
 * whose address was reused by a later mount, meets the same claim.
 *
 * An instance that finds the claim opens nothing. Failing its startup would
 * not do: RunHandler() then clears dn_Task, which by then may be the first
 * instance's, and the next access would start a third. It reports success,
 * leaves dn_Task alone and forwards the packets that reach its own port, which
 * are those of the one caller RunHandler() handed that port to. A DosPacket
 * can be forwarded: the reply goes to dp_Port.
 *
 * A forwarder serves one instance, the one that held the claim when it
 * started, and never takes the volume over: when that instance has gone,
 * dn_Task is NULL again and dos.library starts a fresh instance on the next
 * access, so a forwarder that mounted the volume then would be the second
 * writer this exists to prevent. It looks the claim up for every packet,
 * together with the PutMsg() under Forbid(), so it never holds a pointer to an
 * instance that may have gone. A claim that is missing, or that belongs to a
 * later instance, ends the forwarder: a packet that still names locks or
 * files of the instance that went must not reach another one. That packet is
 * answered ERROR_DEVICE_NOT_MOUNTED. An ACTION_DIE is forwarded like any
 * packet and ends the forwarder too, whatever the instance answers: the
 * sender wants this device's handlers gone, and a forwarder owns nothing
 * that could make it stay.
 *
 * An instance that crashed keeps its claim, and its port is memory of a task
 * that no longer runs. Whoever finds a claim therefore asks whether its owner
 * is still a task of this system. A dead owner's claim is withdrawn: the
 * finder becomes the first instance, a forwarder ends.
 *
 * Forbid() is what makes lookup and publication one step. On an SMP build of
 * exec.library it does not exclude another CPU from the port list; this
 * defence is stated for the uniprocessor kernels AROS ships.
 */
#include <exec/ports.h>
#include <stdint.h>

struct ExecBase;

/* "AFSPLUS." + device name + "." + unit in decimal. */
#define AFSPLUS_CLAIM_DEVICE_NAME_MAX 96
#define AFSPLUS_CLAIM_NAME_BYTES (8 + AFSPLUS_CLAIM_DEVICE_NAME_MAX + 1 + 20 + 1)
#define AFSPLUS_CLAIM_FORWARDERS 8
/* The signal a released claim sends its forwarders: SIGBREAKF_CTRL_C. */
#define AFSPLUS_CLAIM_WAKE_SIGNAL (UINT32_C(1) << 12)

struct Task;

struct AfsplusArosClaim {
    struct MsgPort port;
    struct MsgPort *handler_port;
    struct Task *owner;
    /* Distinguishes this instance from every earlier and later holder of the
     * same name, also when the owner's task address is reused. */
    uint64_t generation;
    /* Forwarders that serve this instance, woken when it releases the claim
     * so that none outlives it idle. A forwarder that finds no free slot
     * still ends with the next packet it cannot forward. */
    struct Task *forwarders[AFSPLUS_CLAIM_FORWARDERS];
    char name[AFSPLUS_CLAIM_NAME_BYTES];
};

/* Supplied by the handler shell: 1 while task is one of the system's tasks.
 * Called under Forbid(). */
uint32_t afsplus_claim_owner_alive(struct ExecBase *sysbase,
    const struct Task *task);

#define AFSPLUS_CLAIM_TAKEN 0
#define AFSPLUS_CLAIM_ALREADY_HELD 1
#define AFSPLUS_CLAIM_NO_MEMORY 2
#define AFSPLUS_CLAIM_NAME_TOO_LONG 3

/* Builds the claim name; 0 when the device name does not fit. */
uint32_t afsplus_claim_name(char *name, const uint8_t *device,
    uint32_t device_length, uint64_t unit);

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

/* Hands message to the live instance that holds the claim name with this
 * generation. Returns 0 when there is none; the message is then still the
 * caller's to answer, and the caller ends. */
uint32_t afsplus_claim_forward(struct ExecBase *sysbase, const char *name,
    uint64_t generation, struct Message *message);

#endif
