/*
 * Copyright (C) 2026 John KNIPPER <code@jkn.me>
 * This file is distributed under the AROS Public License 1.1.
 */
/* Included after the path/lock helpers. No storage operation is emulated:
 * replace calls the filesystem's one transaction, timestamps retain ns. */
static int32_t process_filesystem_v2(struct AfsplusArosPacketContext *context,
    struct FSV2_Request *shared)
{
    struct FSV2_Request request;
    struct AfsplusArosCapabilities capabilities;
    struct AfsplusResolvedParent first = {0}, second = {0};
    struct AfsplusArosFileInfo info;
    struct AfsplusArosStat stat;
    uint8_t name[MAXFILENAMELENGTH];
    uint64_t base, id;
    int64_t now;
    uint32_t ns;
    int32_t error;
    uint32_t size;

    if (!shared) return ERROR_REQUIRED_ARG_MISSING;
    memcpy(&size, &shared->struct_size, sizeof(size));
    if (size != sizeof(request)) return ERROR_BAD_NUMBER;
    memcpy(&request, shared, sizeof(request));
    if (request.object[0] > UINTPTR_MAX || request.object[1] > UINTPTR_MAX)
        return ERROR_INVALID_LOCK;
    if (request.version != FSV2_PACKET_VERSION
        || (request.flags & ~(FSV2_FLAG_FH | FSV2_FLAG_NOFOLLOW)))
        return ERROR_BAD_NUMBER;

    if (request.operation == FSV2_OP_CAPABILITIES) {
        if (request.flags) return ERROR_BAD_NUMBER;
        memset(&capabilities, 0, sizeof(capabilities));
        capabilities.struct_size = sizeof(capabilities);
        error = afsplus_aros_capabilities(context->filesystem, &capabilities);
        if (error) return error;
        request.capabilities = capabilities.capabilities;
        if (require_group(context, AFSPLUS_AROS_GROUP_DOS_METADATA) == 0)
            request.capabilities |= FSV2_CAP_SET_TIME_NS;
        if (require_group(context, AFSPLUS_AROS_GROUP_OBJECT_IDS) == 0)
            request.capabilities |= FSV2_CAP_STAT_NS;
        /* Do not announce paging before the coherence service exists. */
        request.capabilities &= ~FSV2_CAP_PAGING;
    } else if (request.operation == FSV2_OP_STAT) {
        error = require_group(context, AFSPLUS_AROS_GROUP_OBJECT_IDS);
        if (error) return error;
        if (request.flags & FSV2_FLAG_FH) {
            struct AfsplusArosNativeFile *file = find_file(context,
                (BPTR)request.object[0]);
            if (!file || request.path0) return ERROR_INVALID_LOCK;
            error = afsplus_aros_examine_file(context->filesystem, file->id,
                &info, name, sizeof(name));
            if (error) return error;
            id = info.object_id;
        } else {
            error = lock_id(context, (BPTR)request.object[0], &base);
            if (error) return error;
            if (request.path0) {
                const uint8_t *path = (const uint8_t *)request.path0;
                error = resolve_named_object(context, base, path,
                    c_string_length(path), &first);
                if (error) return error;
                if (first.leaf_length) {
                    error = afsplus_aros_lookup_dos_id(context->filesystem,
                        first.id, first.leaf, first.leaf_length, &id);
                } else {
                    error = afsplus_aros_examine_lock(context->filesystem,
                        first.id, &info, name, sizeof(name));
                    id = error ? 0 : info.object_id;
                }
                release_temporary_lock(context, first.id, first.owned);
                if (error) return error;
            } else {
                error = afsplus_aros_examine_lock(context->filesystem, base,
                    &info, name, sizeof(name));
                if (error) return error;
                id = info.object_id;
            }
        }
        memset(&stat, 0, sizeof(stat));
        stat.struct_size = sizeof(stat);
        error = afsplus_aros_stat_id(context->filesystem, id, &stat);
        if (error) return error;
        if (request.path0 && !(request.flags & FSV2_FLAG_NOFOLLOW)
            && stat.kind == 3) return ERROR_IS_SOFT_LINK;
        request.stat.object_id = stat.object_id;
        request.stat.size = stat.size;
        request.stat.allocated_size = stat.allocated_size;
        request.stat.type = stat.kind;
        request.stat.protection = stat.protection;
        request.stat.mtime_seconds = stat.modified_seconds;
        request.stat.mtime_nanoseconds = stat.modified_nanoseconds;
        request.stat.links = stat.links;
        request.stat.ctime_seconds = stat.changed_seconds;
        request.stat.ctime_nanoseconds = stat.changed_nanoseconds;
        request.stat.reserved = 0;
    } else if (request.operation == FSV2_OP_SET_MTIME
        || request.operation == FSV2_OP_REPLACE) {
        const uint8_t *path = (const uint8_t *)request.path0;
        if (!path || (request.flags & FSV2_FLAG_FH))
            return ERROR_BAD_NUMBER;
        error = require_group(context, request.operation == FSV2_OP_REPLACE
            ? AFSPLUS_AROS_GROUP_API_V2 : AFSPLUS_AROS_GROUP_DOS_METADATA);
        if (error) return error;
        error = lock_id(context, (BPTR)request.object[0], &base);
        if (error) return error;
        error = resolve_named_object(context, base, path,
            c_string_length(path), &first);
        if (error) return error;
        if (request.operation == FSV2_OP_REPLACE) {
            if (!request.path1 || request.flags) error = ERROR_BAD_NUMBER;
            if (!error)
                error = lock_id(context, (BPTR)request.object[1], &base);
            if (!error)
                error = resolve_parent(context, base,
                    (const uint8_t *)request.path1,
                    c_string_length((const uint8_t *)request.path1), &second);
            if (!error) error = packet_now(context, &now, &ns);
            if (!error)
                error = afsplus_aros_replace(context->filesystem, first.id,
                    first.leaf, first.leaf_length, second.id, second.leaf,
                    second.leaf_length, now, ns);
            release_temporary_lock(context, second.id, second.owned);
        } else {
            if (request.nanoseconds >= UINT32_C(1000000000))
                error = ERROR_BAD_NUMBER;
            else error = packet_now(context, &now, &ns);
            if (!error)
                error = afsplus_aros_set_modified(context->filesystem,
                    first.id, first.leaf, first.leaf_length, request.seconds,
                    request.nanoseconds, now, ns);
        }
        release_temporary_lock(context, first.id, first.owned);
        if (error) return error;
    } else return ERROR_NOT_IMPLEMENTED;
    memcpy(shared, &request, sizeof(request));
    return 0;
}
