/* SPDX-License-Identifier: BSD-2-Clause */

#include "afsplus_control.h"

#include "afsplus_aros.h"

#include <string.h>

static uint32_t is_separator(char character)
{
    return character == ' ' || character == '\t' || character == ',';
}

/* The decimal number in text, or 0 when it is not one or does not fit. */
static uint32_t parse_count(const char *text, uint32_t length,
    uint32_t limit, uint32_t *value)
{
    uint32_t result = 0;
    uint32_t index;

    if (length == 0 || length > 10)
        return 0;
    for (index = 0; index < length; index++)
    {
        if (text[index] < '0' || text[index] > '9')
            return 0;
        result = result * 10 + (uint32_t)(text[index] - '0');
        if (result > limit)
            return 0;
    }
    *value = result;
    return result != 0;
}

static uint32_t same_word(const char *text, uint32_t length, const char *word)
{
    uint32_t index;

    for (index = 0; index < length; index++)
    {
        char left = text[index];
        char right = word[index];

        if (right == 0)
            return 0;
        if (left >= 'a' && left <= 'z')
            left = (char)(left - 'a' + 'A');
        if (right >= 'a' && right <= 'z')
            right = (char)(right - 'a' + 'A');
        if (left != right)
            return 0;
    }
    return word[length] == 0;
}

uint32_t afsplus_control_parse(const char *text, uint32_t length,
    struct AfsplusArosControl *output)
{
    /* Built aside and copied out only on success, so a string that fails
     * half way leaves the caller with the defaults it was promised. */
    struct AfsplusArosControl parsed;
    uint32_t seen_security = 0;
    uint32_t seen_encoding = 0;
    uint32_t seen_trace = 0;
    uint32_t at = 0;

    parsed.mount_flags = 0;
    parsed.name_encoding = AFSPLUS_AROS_ENCODING_UTF8;
    parsed.trace_events = 0;
    *output = parsed;
    if (text == NULL)
        return AFSPLUS_CONTROL_OK;

    while (at < length)
    {
        uint32_t start;
        uint32_t keyword_length;
        uint32_t value_start;
        const char *keyword;
        const char *value;

        if (is_separator(text[at]))
        {
            at++;
            continue;
        }
        start = at;
        while (at < length && text[at] != '=' && !is_separator(text[at]))
            at++;
        if (at == length || text[at] != '=')
            return AFSPLUS_CONTROL_MALFORMED;
        keyword = text + start;
        keyword_length = at - start;
        at++;
        value_start = at;
        while (at < length && !is_separator(text[at]))
            at++;
        value = text + value_start;
        if (at == value_start)
            return AFSPLUS_CONTROL_MALFORMED;

        if (same_word(keyword, keyword_length, "SECURITY"))
        {
            if (seen_security++)
                return AFSPLUS_CONTROL_REPEATED_KEYWORD;
            if (same_word(value, at - value_start, "PRESERVE"))
                ;
            else if (same_word(value, at - value_start, "STRICT"))
                parsed.mount_flags |=
                    AFSPLUS_AROS_MOUNT_FLAG_STRICT_SECURITY_PROJECTION;
            else if (same_word(value, at - value_start, "DOWNGRADE"))
                parsed.mount_flags |=
                    AFSPLUS_AROS_MOUNT_FLAG_SECURITY_DOWNGRADE;
            else
                return AFSPLUS_CONTROL_UNKNOWN_VALUE;
        }
        else if (same_word(keyword, keyword_length, "ENCODING"))
        {
            if (seen_encoding++)
                return AFSPLUS_CONTROL_REPEATED_KEYWORD;
            if (same_word(value, at - value_start, "UTF8"))
                parsed.name_encoding = AFSPLUS_AROS_ENCODING_UTF8;
            else if (same_word(value, at - value_start, "LATIN1"))
                parsed.name_encoding = AFSPLUS_AROS_ENCODING_LATIN1;
            else
                return AFSPLUS_CONTROL_UNKNOWN_VALUE;
        }
        else if (same_word(keyword, keyword_length, "TRACE"))
        {
            if (seen_trace++)
                return AFSPLUS_CONTROL_REPEATED_KEYWORD;
            if (same_word(value, at - value_start, "OFF"))
                ;
            else if (!parse_count(value, at - value_start,
                AFSPLUS_CONTROL_TRACE_MAX, &parsed.trace_events))
                return AFSPLUS_CONTROL_UNKNOWN_VALUE;
        }
        else
            return AFSPLUS_CONTROL_UNKNOWN_KEYWORD;
    }
    *output = parsed;
    return AFSPLUS_CONTROL_OK;
}
