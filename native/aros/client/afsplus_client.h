/* SPDX-License-Identifier: BSD-2-Clause */

#ifndef AFSPLUS_CLIENT_H
#define AFSPLUS_CLIENT_H

/*
 * Application side of api/afsplus_ext_packet.h: finds the handler behind a
 * file handle, a lock or a path, sends one extension packet, and returns 0 or
 * an ERROR_* value. No call leaves a secondary result in IoErr().
 *
 * A handler without the transport answers ERROR_ACTION_NOT_KNOWN; one that
 * has it never does, and answers ERROR_NOT_IMPLEMENTED for what it cannot
 * do (a missing group of its library, a volume capability, a clock). Only the
 * two calls whose result a classic call can also produce fall back, and only
 * on the first of these answers:
 * afsplus_client_read_at and afsplus_client_write_at. Every other call
 * returns the error, which is the signal to take the portable path: copy
 * instead of clone, write zeros instead of preallocating, do without the
 * report.
 */
#include <dos/dos.h>
#include <exec/ports.h>
#include <stdint.h>

#include "afsplus_aros.h"
#include "afsplus_ext_packet.h"

#ifdef __cplusplus
extern "C" {
#endif

/* The port a request about this object goes to; NULL for a NIL: handle or a
 * zero lock, which name no handler. */
struct MsgPort *afsplus_client_file_port(BPTR file);
struct MsgPort *afsplus_client_lock_port(BPTR lock);

/* The positioned-I/O fallback remembers a few ports that answered "unknown
 * packet" and does not ask them again. A program that knows a handler was
 * replaced clears that memory here; forgetting is never wrong. */
void afsplus_client_forget_ports(void);

/* Sends a filled request. magic, version and header_size are set here. */
LONG afsplus_client_send(struct MsgPort *port,
    struct AfsplusExtRequest *request);

LONG afsplus_client_interface(struct MsgPort *port, uint32_t *revision,
    uint32_t *packet_abi, uint64_t *groups);

/* Positioned I/O that leaves the file position where it was. With the
 * transport the position is never touched. The fallback seeks, transfers and
 * seeks back, so it is not safe against another user of the same handle, and
 * it reaches only offsets a classic Seek can express: beyond that it answers
 * ERROR_OBJECT_TOO_LARGE. If the seek back fails the position is lost and
 * the call answers ERROR_SEEK_ERROR, whatever the transfer did; count still
 * says what was transferred. */
LONG afsplus_client_read_at(BPTR file, uint64_t offset, void *buffer,
    uint32_t length, uint32_t *count);
LONG afsplus_client_write_at(BPTR file, uint64_t offset, const void *buffer,
    uint32_t length, uint32_t *count);

/* Clones the object behind source into target_directory under name, one
 * component in the volume's name encoding. Both locks must belong to the
 * same handler; otherwise ERROR_RENAME_ACROSS_DEVICES and nothing is sent. */
LONG afsplus_client_clone_file(BPTR source, BPTR target_directory,
    CONST_STRPTR name);
LONG afsplus_client_clone_range(BPTR source_file, uint64_t source_offset,
    BPTR target_file, uint64_t target_offset, uint64_t length);
LONG afsplus_client_preallocate(BPTR file, uint64_t offset, uint64_t length);

/* Reports. The struct's struct_size is set here to what this client knows;
 * the handler fills at most that much and stores what it filled. */
LONG afsplus_client_capabilities(struct MsgPort *port,
    struct AfsplusArosCapabilities *output);
LONG afsplus_client_counters(struct MsgPort *port,
    struct AfsplusArosCounters *output);
LONG afsplus_client_health(struct MsgPort *port,
    struct AfsplusArosHealth *output);
/* required receives the size of the whole document; when it exceeds
 * capacity the buffer holds nothing usable and the call still succeeds. */
LONG afsplus_client_info_json(struct MsgPort *port, char *buffer,
    uint32_t capacity, uint32_t *required);

/* Extended attributes of the object behind a lock. Names carry their
 * namespace; this handler writes "user." and "aros.". get and list store the
 * size in required and fill the buffer only when it fits, so a zero capacity
 * asks for the size; list separates names with a NUL after each. mode is an
 * AFSPLUS_AROS_ATTRIBUTE_* value of afsplus_aros.h; REMOVE takes no value. */
LONG afsplus_client_get_attribute(BPTR lock, CONST_STRPTR attribute,
    void *value, uint32_t capacity, uint32_t *required);
LONG afsplus_client_list_attributes(BPTR lock, char *names,
    uint32_t capacity, uint32_t *required);
LONG afsplus_client_set_attribute(BPTR lock, CONST_STRPTR attribute,
    const void *value, uint32_t length, uint32_t mode);

/* Packets by type (AFSPLUS_EXT_COUNT_BY_ACTION) or failures by error code
 * (AFSPLUS_EXT_COUNT_BY_ERROR) since the handler started. stored records are
 * written; total is what the table holds, so a caller with too little room
 * asks again. */
LONG afsplus_client_packet_counts(struct MsgPort *port, uint32_t which,
    struct AfsplusExtPacketCount *records, uint32_t capacity,
    uint32_t *stored, uint32_t *total);

#ifdef __cplusplus
}
#endif

#endif
