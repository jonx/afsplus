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
}

int main(void)
{
    struct AfsplusArosControl control;

    /* No string, an empty one and separators only: the defaults. */
    check(parse(NULL, &control) == AFSPLUS_CONTROL_OK);
    check(control.mount_flags == 0
        && control.name_encoding == AFSPLUS_AROS_ENCODING_UTF8);
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
