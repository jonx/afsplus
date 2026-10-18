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
    uint64_t result = 0;
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
    *value = (uint32_t)result;
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
    uint32_t seen_commit = 0;
    uint32_t seen_cache = 0, seen_backup = 0, seen_restore = 0;
    uint32_t seen_snapshot_edit = 0, seen_snapshot_views = 0;
    uint32_t seen_snapshot_reclaim = 0, seen_format_snapshots = 0;
    uint32_t at = 0;

    parsed.mount_flags = 0;
    parsed.name_encoding = AFSPLUS_AROS_ENCODING_UTF8;
    parsed.trace_events = 0;
    parsed.commit_seconds = AFSPLUS_CONTROL_COMMIT_DEFAULT;
    parsed.commit_named = 0;
    parsed.cache_auto = 1;
    parsed.snapshot_max_edit_records = 0;
    parsed.snapshot_max_views = 0;
    parsed.snapshot_reclaim_records = 0;
    parsed.format_snapshots = 0;
    parsed.backup_policy = 0;
    parsed.restore_policy = 0;
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
        else if (same_word(keyword, keyword_length, "COMMIT"))
        {
            if (seen_commit++)
                return AFSPLUS_CONTROL_REPEATED_KEYWORD;
            parsed.commit_named = 1;
            if (same_word(value, at - value_start, "SYNC"))
                parsed.commit_seconds = 0;
            else if (!parse_count(value, at - value_start,
                AFSPLUS_CONTROL_COMMIT_MAX, &parsed.commit_seconds))
                return AFSPLUS_CONTROL_UNKNOWN_VALUE;
        }
        else if (same_word(keyword, keyword_length, "CACHE"))
        {
            if (seen_cache++)
                return AFSPLUS_CONTROL_REPEATED_KEYWORD;
            if (same_word(value, at - value_start, "AUTO"))
                parsed.cache_auto = 1;
            else if (same_word(value, at - value_start, "BUFFERS"))
                parsed.cache_auto = 0;
            else
                return AFSPLUS_CONTROL_UNKNOWN_VALUE;
        }
        else if (same_word(keyword, keyword_length, "BACKUP"))
        {
            if (seen_backup++) return AFSPLUS_CONTROL_REPEATED_KEYWORD;
            if (same_word(value, at - value_start, "DENY")) parsed.backup_policy = 0;
            else if (same_word(value, at - value_start, "ROOT")) parsed.backup_policy = 1;
            else if (same_word(value, at - value_start, "SINGLEUSER")) parsed.backup_policy = 2;
            else if (same_word(value, at - value_start, "ANYONE")) parsed.backup_policy = 3;
            else return AFSPLUS_CONTROL_UNKNOWN_VALUE;
        }
        else if (same_word(keyword, keyword_length, "RESTORE"))
        {
            if (seen_restore++) return AFSPLUS_CONTROL_REPEATED_KEYWORD;
            if (same_word(value, at - value_start, "DENY")) parsed.restore_policy = 0;
            else if (same_word(value, at - value_start, "ROOT")) parsed.restore_policy = 1;
            else if (same_word(value, at - value_start, "SINGLEUSER")) parsed.restore_policy = 2;
            else if (same_word(value, at - value_start, "ANYONE")) parsed.restore_policy = 3;
            else return AFSPLUS_CONTROL_UNKNOWN_VALUE;
        }
        else if (same_word(keyword, keyword_length, "SNAPSHOTEDIT"))
        {
            if (seen_snapshot_edit++) return AFSPLUS_CONTROL_REPEATED_KEYWORD;
            if (!parse_count(value, at - value_start, UINT32_MAX,
                &parsed.snapshot_max_edit_records)) return AFSPLUS_CONTROL_UNKNOWN_VALUE;
        }
        else if (same_word(keyword, keyword_length, "SNAPSHOTVIEWS"))
        {
            if (seen_snapshot_views++) return AFSPLUS_CONTROL_REPEATED_KEYWORD;
            if (!parse_count(value, at - value_start, UINT32_MAX,
                &parsed.snapshot_max_views)) return AFSPLUS_CONTROL_UNKNOWN_VALUE;
        }
        else if (same_word(keyword, keyword_length, "SNAPSHOTRECLAIM"))
        {
            if (seen_snapshot_reclaim++) return AFSPLUS_CONTROL_REPEATED_KEYWORD;
            if (!parse_count(value, at - value_start, UINT32_MAX,
                &parsed.snapshot_reclaim_records)) return AFSPLUS_CONTROL_UNKNOWN_VALUE;
        }
        else if (same_word(keyword, keyword_length, "FORMATSNAPSHOTS"))
        {
            if (seen_format_snapshots++) return AFSPLUS_CONTROL_REPEATED_KEYWORD;
            if (same_word(value, at - value_start, "YES")) parsed.format_snapshots = 1;
            else if (!same_word(value, at - value_start, "NO"))
                return AFSPLUS_CONTROL_UNKNOWN_VALUE;
        }
        else
            return AFSPLUS_CONTROL_UNKNOWN_KEYWORD;
    }
    if ((seen_snapshot_edit || seen_snapshot_views || seen_snapshot_reclaim)
        && !(seen_snapshot_edit && seen_snapshot_views && seen_snapshot_reclaim))
        return AFSPLUS_CONTROL_MALFORMED;
    if (parsed.snapshot_reclaim_records > parsed.snapshot_max_edit_records
        || (parsed.format_snapshots && !parsed.snapshot_max_views))
        return AFSPLUS_CONTROL_UNKNOWN_VALUE;
    *output = parsed;
    return AFSPLUS_CONTROL_OK;
}
