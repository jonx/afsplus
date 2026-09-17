/* SPDX-License-Identifier: BSD-2-Clause */

#ifndef AFSPLUS_EXT_PACKET_H
#define AFSPLUS_EXT_PACKET_H

/*
 * The transport that carries the 64-bit entry-point groups of afsplus_aros.h
 * from an application to a running handler: one DOS packet type whose first
 * argument is a request block in the sender's memory.
 *
 *   dp_Type  ACTION_AFSPLUS_EXT
 *   dp_Arg1  struct AfsplusExtRequest *
 *   dp_Res1  DOSTRUE or DOSFALSE
 *   dp_Res2  0 or an ERROR_* value
 *
 * A handler that does not know the packet answers ERROR_ACTION_NOT_KNOWN, as
 * every handler does for an unknown type, so a client learns the absence of
 * the transport from the first request and falls back. A handler that knows
 * the packet never answers that value: what it cannot do, for want of an
 * entry-point group, a volume capability or a clock, is
 * ERROR_NOT_IMPLEMENTED. This header uses no
 * AROS type: objects travel as the integers the application already holds.
 *
 * The packet number is provisional. Packet numbers are an AROS-wide
 * allocation; the value spells "AFS2" and lies outside every range
 * dos/dosextens.h assigns. A different allocation changes this one constant.
 */
#include <stddef.h>
#include <stdint.h>

#define ACTION_AFSPLUS_EXT INT32_C(0x41465332)

#define AFSPLUS_EXT_MAGIC UINT32_C(0x41465332)
/* No name component of the format is longer, in any mount encoding. A longer
 * name_length is ERROR_INVALID_COMPONENT_NAME and its bytes are not read. */
#define AFSPLUS_EXT_NAME_MAX UINT32_C(255)
#define AFSPLUS_EXT_VERSION UINT16_C(1)

/* Operations. The group that must be present in the handler's library is
 * named on the right; an operation of an absent group is
 * ERROR_NOT_IMPLEMENTED, an unassigned number ERROR_BAD_NUMBER. */
#define AFSPLUS_EXT_INTERFACE UINT32_C(1)     /* always */
#define AFSPLUS_EXT_CAPABILITIES UINT32_C(2)  /* INTERFACE_QUERY */
#define AFSPLUS_EXT_READ_AT UINT32_C(3)       /* API_V2 */
#define AFSPLUS_EXT_WRITE_AT UINT32_C(4)      /* API_V2 */
#define AFSPLUS_EXT_CLONE_FILE UINT32_C(5)    /* API_V2 */
#define AFSPLUS_EXT_CLONE_RANGE UINT32_C(6)   /* API_V2 */
#define AFSPLUS_EXT_PREALLOCATE UINT32_C(7)   /* API_V2 */
#define AFSPLUS_EXT_REPLACE UINT32_C(8)       /* API_V2 */
#define AFSPLUS_EXT_ADVISE UINT32_C(9)        /* API_V2 */
#define AFSPLUS_EXT_INFO_JSON UINT32_C(10)    /* MANAGE */
#define AFSPLUS_EXT_COUNTERS UINT32_C(11)     /* COUNTERS */
#define AFSPLUS_EXT_HEALTH UINT32_C(12)       /* OBSERVE */
#define AFSPLUS_EXT_EXTENT_MAP UINT32_C(13)   /* EXTENT_MAP */
#define AFSPLUS_EXT_LOOKUP_ID UINT32_C(14)    /* OBJECT_IDS */
#define AFSPLUS_EXT_STAT_ID UINT32_C(15)      /* OBJECT_IDS */
#define AFSPLUS_EXT_PACKET_COUNTS UINT32_C(16) /* always */
#define AFSPLUS_EXT_GET_ATTRIBUTE UINT32_C(17)   /* ATTRIBUTES */
#define AFSPLUS_EXT_LIST_ATTRIBUTES UINT32_C(18) /* ATTRIBUTES */
#define AFSPLUS_EXT_SET_ATTRIBUTE UINT32_C(19)   /* ATTRIBUTES */

/* One record of AFSPLUS_EXT_PACKET_COUNTS. With flags 0 the key is a packet
 * type, count the packets of that type answered since the handler started
 * and failed those among them answered with an error. With flags 1 the key
 * is an ERROR_* value and count the packets answered with it; failed equals
 * count. The tables are bounded: what no longer fits is summed under
 * AFSPLUS_EXT_COUNT_OTHER, which is then the last record. */
#define AFSPLUS_EXT_COUNT_BY_ACTION UINT32_C(0)
#define AFSPLUS_EXT_COUNT_BY_ERROR UINT32_C(1)
#define AFSPLUS_EXT_COUNT_OTHER INT32_MAX

struct AfsplusExtPacketCount {
    int32_t key;
    uint32_t reserved;
    uint64_t count;
    uint64_t failed;
};

/* A pointer that occupies eight bytes on every target, so the block has one
 * layout. A 32-bit sender clears the block first. */
#define AFSPLUS_EXT_POINTER(type, name) \
    union { type *name; uint64_t name##_bits; }

/*
 * One block for every operation; an operation reads the fields listed for
 * it. reserved, and flags where the operation does not list it, must be zero
 * (ERROR_BAD_NUMBER), so that they can be given a meaning later; the other
 * unlisted fields are ignored. The sender clears the block, fills magic, version
 * and header_size = sizeof(struct AfsplusExtRequest), and the inputs.
 *
 * object[]: a lock as the BPTR the application holds (0: the volume root),
 * or a file as the fh_Arg1 of its FileHandle. Both must belong to the
 * handler that receives the packet.
 *
 *   INTERFACE     -> output_count revision, output_flags packet ABI,
 *                    output_value groups
 *   CAPABILITIES  buffer: struct AfsplusArosCapabilities, struct_size set
 *   READ_AT       object[0] file, offset[0], buffer, buffer_size
 *                 -> output_count bytes read
 *   WRITE_AT      object[0] file, offset[0], buffer, buffer_size
 *                 -> output_count bytes written
 *   CLONE_FILE    object[0] source lock, object[1] target base lock, name[1]
 *   CLONE_RANGE   object[0] source file, offset[0], object[1] target file,
 *                 offset[1], length
 *   PREALLOCATE   object[0] file, offset[0], length
 *   REPLACE       object[0] source base lock, name[0], object[1] target base
 *                 lock, name[1]
 *   ADVISE        object[0] file, offset[0], length, flags: hint
 *                 -> output_flags effect
 *   INFO_JSON     buffer, buffer_size -> output_value bytes required
 *   COUNTERS      buffer: struct AfsplusArosCounters, struct_size set
 *   HEALTH        buffer: struct AfsplusArosHealth, struct_size set
 *   EXTENT_MAP    object[0] file, offset[0], length, buffer: array of struct
 *                 AfsplusArosExtent, buffer_size in bytes
 *                 -> output_count extents, output_flags complete,
 *                    output_value next offset
 *   LOOKUP_ID     object[0] base lock, name[0] -> output_value object ID
 *   STAT_ID       offset[0]: object ID, buffer: struct AfsplusArosStat,
 *                 struct_size set
 *   GET_ATTRIBUTE object[0] base lock, name[0] object, name[1] attribute,
 *                 buffer, buffer_size -> output_value bytes required; the
 *                 buffer is filled only when the value fits
 *   LIST_ATTRIBUTES object[0] base lock, name[0] object, buffer, buffer_size
 *                 -> output_value bytes required; names each followed by NUL
 *   SET_ATTRIBUTE object[0] base lock, name[0] object, name[1] attribute,
 *                 buffer, buffer_size: the value, flags: an
 *                 AFSPLUS_AROS_ATTRIBUTE_* mode of afsplus_aros.h
 *   PACKET_COUNTS flags: which table, buffer: array of struct
 *                 AfsplusExtPacketCount, buffer_size in bytes
 *                 -> output_count records stored, output_value records the
 *                    table holds; the request itself is counted by the next
 *
 * A struct buffer must be at least as large as the struct_size it declares.
 * Names are single components in the mount's name encoding, exactly as the
 * entry points of afsplus_aros.h take them; the transport resolves no path.
 */
struct AfsplusExtRequest {
    uint32_t magic;
    uint16_t version;
    uint16_t header_size;
    uint32_t operation;
    uint32_t flags;
    uint64_t object[2];
    uint64_t offset[2];
    uint64_t length;
    AFSPLUS_EXT_POINTER(const uint8_t, name0);
    AFSPLUS_EXT_POINTER(const uint8_t, name1);
    uint32_t name_length[2];
    AFSPLUS_EXT_POINTER(void, buffer);
    uint32_t buffer_size;
    uint32_t output_count;
    uint32_t output_flags;
    uint32_t reserved;
    uint64_t output_value;
};

/* One EXTENT_MAP request stores at most this many extents, whatever the
 * buffer holds; output_flags and output_value say whether and where to
 * continue. */
#define AFSPLUS_EXT_EXTENTS_MAX UINT32_C(64)

/* magic, version and header_size: all a handler reads of a block before
 * header_size has told it that more exists. */
#define AFSPLUS_EXT_PREFIX_BYTES 8

#if defined(__STDC_VERSION__) && __STDC_VERSION__ >= 201112L
_Static_assert(offsetof(struct AfsplusExtRequest, operation)
    == AFSPLUS_EXT_PREFIX_BYTES, "AfsplusExtRequest prefix drift");
_Static_assert(sizeof(struct AfsplusExtRequest) == 112,
    "AfsplusExtRequest layout drift");
_Static_assert(sizeof(struct AfsplusExtPacketCount) == 24,
    "AfsplusExtPacketCount layout drift");
#endif

#endif
