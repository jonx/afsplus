/* Copyright (C) 2026 John KNIPPER <code@jkn.me>
 * Licensed under the AROS Public License (APL) Version 1.1.
 */
#include "../../api/filesystem_paging_v2.h"

/* A read-only mount, or a profile without reflinks, cannot create an orphan.
 * Its explicit copy fallback runs inside one serialized handler dispatch.
 * Device completion callbacks transfer blocks only; they do not mutate the
 * mounted Volume. No other handler packet can write/truncate during copying.
 * These buffers are snapshot backing, not eagerly resident mapping pages.
 */
#define AFSPLUS_SNAPSHOT_CHUNK 65536U
struct AfsplusPagingChunk {
    struct AfsplusPagingChunk *next;
    uint32_t length;
    uint8_t bytes[AFSPLUS_SNAPSHOT_CHUNK];
};
struct AfsplusPagingSnapshot {
    uint64_t size;
    uint32_t references;
    struct AfsplusPagingChunk *chunks;
    struct AfsplusArosFileInfo info;
    uint8_t name[MAXFILENAMELENGTH];
};

static void paging_release(struct AfsplusArosPacketContext *context,
    struct AfsplusPagingSnapshot *snapshot)
{
    struct AfsplusPagingChunk *chunk;
    if (--snapshot->references != 0)
        return;
    chunk = snapshot->chunks;
    while (chunk != NULL) {
        struct AfsplusPagingChunk *next = chunk->next;
        context->free(context->callback_context, chunk, sizeof(*chunk));
        chunk = next;
    }
    context->free(context->callback_context, snapshot, sizeof(*snapshot));
}

static int32_t paging_copy(struct AfsplusArosPacketContext *context,
    struct AfsplusArosNativeFile *source, struct AfsplusPagingSnapshot **output)
{
    struct AfsplusPagingSnapshot *snapshot;
    struct AfsplusPagingChunk **tail;
    uint64_t offset = 0;
    int32_t error;
    snapshot = context->allocate(context->callback_context, sizeof(*snapshot));
    if (snapshot == NULL)
        return ERROR_NO_FREE_STORE;
    memset(snapshot, 0, sizeof(*snapshot));
    snapshot->references = 1;
    error = afsplus_aros_file_size(context->filesystem, source->id,
        &snapshot->size);
    if (error == 0)
        error = afsplus_aros_examine_file(context->filesystem, source->id,
            &snapshot->info, snapshot->name, sizeof(snapshot->name));
    tail = &snapshot->chunks;
    while (error == 0 && offset < snapshot->size) {
        struct AfsplusPagingChunk *chunk = context->allocate(
            context->callback_context, sizeof(*chunk));
        uint32_t wanted = snapshot->size - offset > AFSPLUS_SNAPSHOT_CHUNK
            ? AFSPLUS_SNAPSHOT_CHUNK : (uint32_t)(snapshot->size - offset);
        uint32_t done = 0;
        if (chunk == NULL) { error = ERROR_NO_FREE_STORE; break; }
        chunk->next = NULL;
        chunk->length = wanted;
        *tail = chunk;
        tail = &chunk->next;
        while (error == 0 && done < wanted) {
            uint32_t count = 0;
            error = afsplus_aros_read_at(context->filesystem, source->id,
                offset + done, chunk->bytes + done, wanted - done, &count);
            if (error == 0 && (count == 0 || count > wanted - done))
                error = ERROR_SEEK_ERROR;
            if (error == 0)
                done += count;
        }
        offset += done;
    }
    if (error != 0) { paging_release(context, snapshot); return error; }
    *output = snapshot;
    return 0;
}

static void paging_read(struct AfsplusArosNativeFile *file, uint8_t *buffer,
    uint32_t wanted, uint32_t *count)
{
    struct AfsplusPagingChunk *chunk = file->snapshot->chunks;
    uint64_t offset = file->snapshot_position;
    uint32_t done = 0;
    while (chunk != NULL && offset >= chunk->length) {
        offset -= chunk->length;
        chunk = chunk->next;
    }
    while (chunk != NULL && done < wanted) {
        uint32_t part = chunk->length - (uint32_t)offset;
        if (part > wanted - done) part = wanted - done;
        memcpy(buffer + done, chunk->bytes + (size_t)offset, part);
        done += part;
        offset = 0;
        chunk = chunk->next;
    }
    file->snapshot_position += done;
    *count = done;
}

static int32_t process_paging_v2(struct AfsplusArosPacketContext *context,
    struct FSV2_PagingRequest *shared)
{
    struct FSV2_PagingRequest request;
    struct AfsplusArosNativeFile *source, *file;
    struct FileHandle *destination;
    uint64_t id = 0, size = 0;
    int64_t seconds;
    uint32_t nanoseconds;
    int32_t error = 0;
    if (shared == NULL) return ERROR_REQUIRED_ARG_MISSING;
    memcpy(&request, shared, 8);
    if (request.struct_size != sizeof(request)
        || request.version != FSV2_PAGING_VERSION
        || request.operation != FSV2_PAGING_SNAPSHOT_OPEN)
        return ERROR_NOT_IMPLEMENTED;
    memcpy((uint8_t *)&request + 8, (const uint8_t *)shared + 8,
        sizeof(request) - 8);
    if ((request.flags & ~FSV2_SNAPSHOT_ALLOW_COPY) != 0
        || request.reserved != 0 || request.reserved2 != 0
        || request.source > UINTPTR_MAX || request.destination == NULL)
        return ERROR_BAD_NUMBER;
    source = find_file(context, (BPTR)(uintptr_t)request.source);
    if (source == NULL) return ERROR_INVALID_LOCK;
    file = reserve_file(context, 0, &error);
    if (file == NULL) return error;
    destination = request.destination;
    if (source->snapshot != NULL) {
        if (source->snapshot->references == UINT32_MAX)
            error = ERROR_NO_FREE_STORE;
        else {
            file->snapshot = source->snapshot;
            file->snapshot->references++;
            size = file->snapshot->size;
        }
    } else {
        /* Old FFI libraries must not silently provide a live source handle. */
        if (context->revision < 22
            || require_group(context, AFSPLUS_AROS_GROUP_API_V2) != 0)
            error = ERROR_NOT_IMPLEMENTED;
        else {
            error = packet_now(context, &seconds, &nanoseconds);
            if (error == 0)
                error = afsplus_aros_clone_open_snapshot(context->filesystem,
                    source->id, seconds, nanoseconds, &id);
        }
        if ((error == ERROR_ACTION_NOT_KNOWN
                || error == ERROR_DISK_WRITE_PROTECTED)
            && (request.flags & FSV2_SNAPSHOT_ALLOW_COPY) != 0) {
            error = paging_copy(context, source, &file->snapshot);
            if (error == 0) size = file->snapshot->size;
        } else if (error == 0)
            error = afsplus_aros_file_size(context->filesystem, id, &size);
    }
    if (error != 0) {
        if (id != 0) (void)afsplus_aros_close(context->filesystem, id);
        if (file->snapshot != NULL) paging_release(context, file->snapshot);
        discard_reserved_file(context, file);
        return error == ERROR_ACTION_NOT_KNOWN ? ERROR_NOT_IMPLEMENTED : error;
    }
    publish_file(context, file, id);
    destination->fh_Arg1 = (SIPTR)MKBADDR(file);
    destination->fh_Type = context->handler_port;
    destination->fh_Port = DOSFALSE;
    shared->size = size;
    shared->storage = file->snapshot != NULL ? FSV2_SNAPSHOT_RAM
        : FSV2_SNAPSHOT_REFLINK;
    return 0;
}

/* Intercept only operations whose first argument is an open file cookie.
 * Every other packet follows the existing lock/path dispatch unchanged. */
static int paging_file_packet(struct AfsplusArosPacketContext *context,
    struct DosPacket *packet, uint32_t packet64,
    SIPTR *result, int64_t *result64, int32_t *error)
{
    struct AfsplusArosNativeFile *file;
    int64_t offset = (int64_t)packet->dp_Arg2;
    LONG mode = (LONG)packet->dp_Arg3;
    switch (packet->dp_Type) {
    case ACTION_READ: case ACTION_WRITE: case ACTION_END:
    case ACTION_SEEK: case ACTION_SEEK64: case ACTION_CHANGE_FILE_POSITION64:
    case ACTION_SET_FILE_SIZE: case ACTION_SET_FILE_SIZE64:
    case ACTION_CHANGE_FILE_SIZE64: case ACTION_GET_FILE_SIZE64:
    case ACTION_GET_FILE_POSITION64: case ACTION_EXAMINE_FH:
    case ACTION_EXAMINE_FH64: break;
    default: return 0;
    }
    file = find_file(context, (BPTR)packet->dp_Arg1);
    if (file == NULL || file->snapshot == NULL) return 0;
#if (__WORDSIZE != 64)
    if (packet64) {
        offset = (int64_t)((struct DosPacket64 *)packet)->dp_Arg2;
        mode = (LONG)((struct DosPacket64 *)packet)->dp_Arg3;
    }
    if (packet->dp_Type == ACTION_SEEK64) {
        *error = ERROR_ACTION_NOT_KNOWN;
        *result = *result64 = -1;
        return 1;
    }
#else
    (void)packet64;
#endif
    *error = 0;
    switch (packet->dp_Type) {
    case ACTION_READ: {
        uint32_t count = 0;
        if (packet->dp_Arg3 < 0 || (uint64_t)packet->dp_Arg3 > UINT32_MAX
            || (packet->dp_Arg3 != 0 && packet->dp_Arg2 == 0))
            *error = ERROR_BAD_NUMBER;
        else
            paging_read(file, (uint8_t *)packet->dp_Arg2,
                (uint32_t)packet->dp_Arg3, &count);
        *result64 = *result = *error == 0 ? (SIPTR)count : -1;
        break;
    }
    case ACTION_END:
        paging_release(context, file->snapshot);
        unlink_file(context, file);
        *result64 = *result = DOSTRUE;
        break;
    case ACTION_WRITE: case ACTION_SET_FILE_SIZE: case ACTION_SET_FILE_SIZE64:
    case ACTION_CHANGE_FILE_SIZE64:
        *error = ERROR_DISK_WRITE_PROTECTED;
        *result64 = *result = -1;
        break;
    case ACTION_GET_FILE_SIZE64: case ACTION_GET_FILE_POSITION64: {
        uint64_t value = packet->dp_Type == ACTION_GET_FILE_SIZE64
            ? file->snapshot->size : file->snapshot_position;
        if (value > INT64_MAX) *error = ERROR_OBJECT_TOO_LARGE;
        *result64 = *error == 0 ? (int64_t)value : -1;
        *result = (SIPTR)*result64;
        break;
    }
    case ACTION_EXAMINE_FH: case ACTION_EXAMINE_FH64:
        *error = fill_packet_fib(packet->dp_Type, (BPTR)packet->dp_Arg2,
            &file->snapshot->info, file->snapshot->name, NULL, 0);
        *result64 = *result = *error == 0 ? DOSTRUE : DOSFALSE;
        break;
    default: {
        uint64_t base = mode == OFFSET_BEGINNING ? 0
            : mode == OFFSET_CURRENT ? file->snapshot_position
            : mode == OFFSET_END ? file->snapshot->size : UINT64_MAX;
        uint64_t old = file->snapshot_position, next = 0;
        uint64_t magnitude = offset < 0 ? (uint64_t)(-(offset + 1)) + 1
            : (uint64_t)offset;
        if (base == UINT64_MAX || (offset < 0 && magnitude > base)
            || (offset >= 0 && magnitude > UINT64_MAX - base))
            *error = ERROR_SEEK_ERROR;
        else next = offset < 0 ? base - magnitude : base + magnitude;
        if (*error == 0 && (next > INT64_MAX
                || (packet->dp_Type == ACTION_SEEK && old > INT32_MAX)))
            *error = ERROR_OBJECT_TOO_LARGE;
        if (*error == 0) file->snapshot_position = next;
        *result64 = *error != 0 ? -1
            : packet->dp_Type == ACTION_CHANGE_FILE_POSITION64
                ? DOSTRUE : (int64_t)old;
        *result = (SIPTR)*result64;
        break;
    }
    }
    return 1;
}
