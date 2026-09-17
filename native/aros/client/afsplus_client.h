/* SPDX-License-Identifier: BSD-2-Clause */

#ifndef AFSPLUS_CLIENT_H
#define AFSPLUS_CLIENT_H

/*
 * Application side of api/afsplus_ext_packet.h: finds the handler behind a
 * file handle, a lock or a path, sends one extension packet, and returns 0 or
 * an ERROR_* value. No call leaves a secondary result in IoErr().
 *
 * A handler without the transport answers ERROR_ACTION_NOT_KNOWN. Only the
 * two calls whose result a classic call can also produce fall back on it:
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

/* Sends a filled request. magic, version and header_size are set here. */
LONG afsplus_client_send(struct MsgPort *port,
    struct AfsplusExtRequest *request);

LONG afsplus_client_interface(struct MsgPort *port, uint32_t *revision,
    uint32_t *packet_abi, uint64_t *groups);

/* Positioned I/O that leaves the file position where it was. With the
 * transport the position is never touched. The fallback seeks, transfers and
 * seeks back, so it is not safe against another user of the same handle, and
 * it reaches only offsets a classic Seek can express: beyond that it answers
 * ERROR_OBJECT_TOO_LARGE. */
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

#ifdef __cplusplus
}
#endif

#endif
