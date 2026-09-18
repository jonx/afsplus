/* SPDX-License-Identifier: BSD-2-Clause */

#ifndef AFSPLUS_AROS_CONTROL_H
#define AFSPLUS_AROS_CONTROL_H

/*
 * The DOSDriver Control string.
 *
 * A mount is described by a DOSDriver whose environment carries a free-form
 * Control string (DE_CONTROL). It is the only place where a real system can
 * say anything about a mount that the numeric fields do not cover, so the
 * handler's two security policies and its name encoding, which were reachable
 * only from a program that called the C boundary directly, are set here.
 *
 * The syntax is the usual one: settings separated by spaces or commas, each
 * KEYWORD=VALUE, both compared without regard to case. An unknown keyword or
 * value FAILS THE MOUNT rather than being ignored: a Control string that asks
 * for a policy the handler does not know must not look as if it had been
 * applied.
 *
 *   SECURITY=PRESERVE   a classic protection write on an object carrying an
 *                       on-disk security descriptor lands, every byte of the
 *                       descriptor stays and the projection is marked as
 *                       diverged. The default.
 *   SECURITY=STRICT     that write is refused instead (ERROR_WRITE_PROTECTED).
 *   SECURITY=DOWNGRADE  it may also replace security metadata the container
 *                       does not hold.
 *   ENCODING=UTF8       names on the wire are UTF-8. The default.
 *   ENCODING=LATIN1     names on the wire are Latin-1, converted on the way
 *                       in and out; what a classic program sends.
 *   TRACE=OFF           no trace ring, and the core records nothing. The
 *                       default: the ring costs memory and a clock read per
 *                       event.
 *   TRACE=<n>           keep a ring of n events, 1 to 4096, which a tool
 *                       drains through the extension packet.
 *   COMMIT=<seconds>    changes gather and are committed together after one
 *                       idle second, or when the oldest is this old, 1 to
 *                       60 (ADR-121); a crash loses at most those changes,
 *                       whole. COMMIT=5 is the default.
 *   COMMIT=SYNC         every change is durable when its packet is answered.
 */
#include <stdint.h>

#define AFSPLUS_CONTROL_TRACE_MAX 4096
#define AFSPLUS_CONTROL_COMMIT_MAX 60
#define AFSPLUS_CONTROL_COMMIT_DEFAULT 5

struct AfsplusArosControl {
    uint32_t mount_flags;   /* AFSPLUS_AROS_MOUNT_FLAG_* of afsplus_aros.h */
    uint32_t name_encoding; /* AFSPLUS_AROS_ENCODING_* of afsplus_aros.h */
    uint32_t trace_events;  /* ring size, 0 for no trace ring */
    uint32_t commit_seconds; /* longest a change waits, 0 for SYNC */
    uint32_t commit_named;   /* 1 when the string said COMMIT= */
};

#define AFSPLUS_CONTROL_OK 0
#define AFSPLUS_CONTROL_UNKNOWN_KEYWORD 1
#define AFSPLUS_CONTROL_UNKNOWN_VALUE 2
#define AFSPLUS_CONTROL_REPEATED_KEYWORD 3
#define AFSPLUS_CONTROL_MALFORMED 4

/* Fills output with the defaults and applies text. Returns an
 * AFSPLUS_CONTROL_* code; on anything but OK the output is the defaults and
 * the caller must fail the mount. A NULL or empty string is OK. */
uint32_t afsplus_control_parse(const char *text, uint32_t length,
    struct AfsplusArosControl *output);

#endif
