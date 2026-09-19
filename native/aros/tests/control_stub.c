/* SPDX-License-Identifier: BSD-2-Clause */

/* Host matrix of the DOSDriver Control string parser: what a mountlist can
 * say, and what it cannot say silently. */

#include "afsplus_control.h"

#include "afsplus_aros.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define check(condition) \
    do { \
        if (!(condition)) \
        { \
            printf("assertion failed: %s (%s:%d)\n", #condition, \
                __FILE__, __LINE__); \
            fflush(NULL); \
            abort(); \
        } \
    } while (0)

static uint32_t parse(const char *text, struct AfsplusArosControl *output)
{
    return afsplus_control_parse(text, text != NULL
        ? (uint32_t)strlen(text) : 0, output);
}

/* Every refusal leaves the defaults, whatever it had already read. */
static void refuses(const char *text, uint32_t reason)
{
    struct AfsplusArosControl control;
    uint32_t result = parse(text, &control);

    check(result == reason);
    check(control.mount_flags == 0);
    check(control.name_encoding == AFSPLUS_AROS_ENCODING_UTF8);
    check(control.trace_events == 0);
    check(control.commit_seconds == AFSPLUS_CONTROL_COMMIT_DEFAULT
        && control.commit_named == 0);
    check(control.cache_auto == 1);
}

int main(void)
{
    struct AfsplusArosControl control;

    /* No string, an empty one and separators only: the defaults. */
    check(parse(NULL, &control) == AFSPLUS_CONTROL_OK);
    check(control.mount_flags == 0
        && control.name_encoding == AFSPLUS_AROS_ENCODING_UTF8
        && control.trace_events == 0);
    check(parse("", &control) == AFSPLUS_CONTROL_OK);
    check(parse("   ,, \t ", &control) == AFSPLUS_CONTROL_OK);
    check(control.mount_flags == 0);

    /* The two security policies that had no way in before. */
    check(parse("SECURITY=STRICT", &control) == AFSPLUS_CONTROL_OK);
    check(control.mount_flags
        == AFSPLUS_AROS_MOUNT_FLAG_STRICT_SECURITY_PROJECTION);
    check(parse("SECURITY=DOWNGRADE", &control) == AFSPLUS_CONTROL_OK);
    check(control.mount_flags == AFSPLUS_AROS_MOUNT_FLAG_SECURITY_DOWNGRADE);
    /* Named explicitly, the default is the default, and never both bits. */
    check(parse("SECURITY=PRESERVE", &control) == AFSPLUS_CONTROL_OK);
    check(control.mount_flags == 0);

    check(parse("ENCODING=LATIN1", &control) == AFSPLUS_CONTROL_OK);
    check(control.name_encoding == AFSPLUS_AROS_ENCODING_LATIN1
        && control.mount_flags == 0);
    check(parse("ENCODING=UTF8", &control) == AFSPLUS_CONTROL_OK);
    check(control.name_encoding == AFSPLUS_AROS_ENCODING_UTF8);

    /* Case does not matter, nor which separator, nor the order. */
    check(parse("encoding=latin1 security=Strict", &control)
        == AFSPLUS_CONTROL_OK);
    check(control.name_encoding == AFSPLUS_AROS_ENCODING_LATIN1);
    check(control.mount_flags
        == AFSPLUS_AROS_MOUNT_FLAG_STRICT_SECURITY_PROJECTION);
    check(parse(" ,SECURITY=downgrade,\tENCODING=LATIN1 ", &control)
        == AFSPLUS_CONTROL_OK);
    check(control.mount_flags == AFSPLUS_AROS_MOUNT_FLAG_SECURITY_DOWNGRADE);
    check(control.name_encoding == AFSPLUS_AROS_ENCODING_LATIN1);

    /* The trace ring: off by default, a count when asked for, and bounded. */
    check(parse("TRACE=OFF", &control) == AFSPLUS_CONTROL_OK);
    check(control.trace_events == 0);
    check(parse("TRACE=256", &control) == AFSPLUS_CONTROL_OK);
    check(control.trace_events == 256);
    check(parse("trace=1", &control) == AFSPLUS_CONTROL_OK);
    check(control.trace_events == 1);
    {
        char largest[32];

        sprintf(largest, "TRACE=%u", (unsigned)AFSPLUS_CONTROL_TRACE_MAX);
        check(parse(largest, &control) == AFSPLUS_CONTROL_OK);
        check(control.trace_events == AFSPLUS_CONTROL_TRACE_MAX);
        sprintf(largest, "TRACE=%u", (unsigned)AFSPLUS_CONTROL_TRACE_MAX + 1);
        refuses(largest, AFSPLUS_CONTROL_UNKNOWN_VALUE);
    }
    refuses("TRACE=0", AFSPLUS_CONTROL_UNKNOWN_VALUE);
    refuses("TRACE=-1", AFSPLUS_CONTROL_UNKNOWN_VALUE);
    refuses("TRACE=12x", AFSPLUS_CONTROL_UNKNOWN_VALUE);
    refuses("TRACE=99999999999", AFSPLUS_CONTROL_UNKNOWN_VALUE);
    refuses("TRACE=ON", AFSPLUS_CONTROL_UNKNOWN_VALUE);
    refuses("TRACE=256 TRACE=256", AFSPLUS_CONTROL_REPEATED_KEYWORD);

    /* Delayed commit: five seconds unless the string says otherwise. */
    check(parse(NULL, &control) == AFSPLUS_CONTROL_OK);
    check(control.commit_seconds == 5 && control.commit_named == 0);
    check(parse("COMMIT=SYNC", &control) == AFSPLUS_CONTROL_OK);
    check(control.commit_seconds == 0 && control.commit_named == 1);
    check(parse("commit=30 trace=8", &control) == AFSPLUS_CONTROL_OK);
    check(control.commit_seconds == 30 && control.commit_named == 1
        && control.trace_events == 8);
    check(parse("COMMIT=60", &control) == AFSPLUS_CONTROL_OK);
    check(control.commit_seconds == 60);
    refuses("COMMIT=61", AFSPLUS_CONTROL_UNKNOWN_VALUE);
    refuses("COMMIT=0", AFSPLUS_CONTROL_UNKNOWN_VALUE);
    refuses("COMMIT=NEVER", AFSPLUS_CONTROL_UNKNOWN_VALUE);
    refuses("COMMIT=5 COMMIT=SYNC", AFSPLUS_CONTROL_REPEATED_KEYWORD);

    /* The read cache: sized by memory unless the string pins Buffers. */
    check(parse(NULL, &control) == AFSPLUS_CONTROL_OK);
    check(control.cache_auto == 1);
    check(parse("CACHE=BUFFERS", &control) == AFSPLUS_CONTROL_OK);
    check(control.cache_auto == 0);
    check(parse("cache=auto,COMMIT=SYNC", &control) == AFSPLUS_CONTROL_OK);
    check(control.cache_auto == 1 && control.commit_seconds == 0);
    refuses("CACHE=BUFFERS COMMIT=61", AFSPLUS_CONTROL_UNKNOWN_VALUE);
    refuses("CACHE=64", AFSPLUS_CONTROL_UNKNOWN_VALUE);
    refuses("CACHE=AUTO CACHE=BUFFERS", AFSPLUS_CONTROL_REPEATED_KEYWORD);

    /* What a mountlist may not say without being told. */
    refuses("SECURITY=NONE", AFSPLUS_CONTROL_UNKNOWN_VALUE);
    refuses("ENCODING=UTF16", AFSPLUS_CONTROL_UNKNOWN_VALUE);
    refuses("SECURITYX=STRICT", AFSPLUS_CONTROL_UNKNOWN_KEYWORD);
    refuses("SECURIT=STRICT", AFSPLUS_CONTROL_UNKNOWN_KEYWORD);
    refuses("CASE=SENSITIVE", AFSPLUS_CONTROL_UNKNOWN_KEYWORD);
    refuses("SECURITY", AFSPLUS_CONTROL_MALFORMED);
    refuses("SECURITY=", AFSPLUS_CONTROL_MALFORMED);
    refuses("=STRICT", AFSPLUS_CONTROL_UNKNOWN_KEYWORD);
    refuses("SECURITY=STRICT SECURITY=PRESERVE",
        AFSPLUS_CONTROL_REPEATED_KEYWORD);
    refuses("ENCODING=LATIN1 ENCODING=LATIN1",
        AFSPLUS_CONTROL_REPEATED_KEYWORD);
    /* A good setting followed by a bad one applies neither. */
    refuses("ENCODING=LATIN1 SECURITY=NONE", AFSPLUS_CONTROL_UNKNOWN_VALUE);
    refuses("SECURITY=STRICT NONSENSE=1", AFSPLUS_CONTROL_UNKNOWN_KEYWORD);

    /* A keyword that is a prefix of a known one, and one that extends it. */
    refuses("ENCODING=LATIN", AFSPLUS_CONTROL_UNKNOWN_VALUE);
    refuses("ENCODING=LATIN11", AFSPLUS_CONTROL_UNKNOWN_VALUE);

    /* The length is what bounds the text, not a terminator. */
    check(afsplus_control_parse("SECURITY=STRICTX", 15, &control)
        == AFSPLUS_CONTROL_OK);
    check(control.mount_flags
        == AFSPLUS_AROS_MOUNT_FLAG_STRICT_SECURITY_PROJECTION);
    check(afsplus_control_parse("SECURITY=STRICT", 8, &control)
        == AFSPLUS_CONTROL_MALFORMED);

    puts("afsplus control stub: PASS");
    return 0;
}
