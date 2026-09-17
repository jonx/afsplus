/* SPDX-License-Identifier: BSD-2-Clause */

#include "afsplus_packet.h"

#include <dos/dosasl.h>
#include <dos/exall.h>

#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define STUB_FILESYSTEM ((struct AfsplusAros *)(uintptr_t)UINT32_C(0x1000))
#define MAX_EVENTS 64

struct StubEvent {
    char operation;
    uint64_t base;
    uint32_t access;
    uint32_t length;
    uint8_t name[32];
};

static struct StubEvent events[MAX_EVENTS];
static size_t event_count;
static uint64_t next_lock = 10;
static uint64_t next_file = 100;
static uint64_t file_position;
static uint64_t file_size;
static uint32_t flush_count;
static uint32_t fsync_count;
static uint32_t close_count;
static uint32_t examine_count;
static uint32_t fail_allocations;
static uint64_t created_lock_id;
/* Negative: examine_next yields "entry" forever (the single-entry cases).
 * Otherwise a directory of that many entries named e0, e1, ... */
static int32_t stub_directory_size = -1;
static int32_t stub_directory_at;
static uint32_t rewind_count;
/* Index at which examine_next fails once with ERROR_SEEK_ERROR, or -1. */
static int32_t stub_directory_fail_at = -1;
static uint64_t stub_next_watch = 500;
static uint64_t stub_fired[4];
static uint32_t stub_fired_count;
static uint32_t stub_removed_watches;
/* Registered by the notify cases and still live when ACTION_DIE is tried. */
static struct NotifyRequest second;
static struct NotifyRequest *delivered[8];
static uint32_t delivered_count;
static uint64_t stub_groups = UINT64_C(0x7FFF);
static uint32_t stub_revision = AFSPLUS_AROS_INTERFACE_REVISION;
static uint32_t stub_protect;
static uint32_t stub_protect_key;
static int32_t stub_label_error;
static uint8_t relabelled[16];
static uint32_t relabelled_length;
static uint32_t relabel_calls;
static char relabel_phases[8];
static int32_t stub_relabel_prepare_error;
static int32_t stub_record_error;
/* Lets a free succeed while locks still collide. */
static uint32_t stub_free_succeeds;
static uint64_t stub_record_offset;
static uint64_t stub_record_length;
static uint32_t stub_record_exclusive;
static int32_t stub_open_from_lock_error;
static int32_t stub_change_mode_error;
static uint32_t stub_changed_access;
static const char *stub_link_target = "";
static uint32_t stub_protection;
static int64_t stub_modified_seconds;
static uint32_t stub_modified_nanoseconds;

void __assert(const char *expression, const char *file, unsigned int line)
{
    printf("assertion failed: %s (%s:%u)\n", expression, file, line);
    fflush(NULL);
    abort();
}

static void record(char operation, uint64_t base, const uint8_t *name,
    uint32_t length, uint32_t access)
{
    struct StubEvent *event;

    assert(event_count < MAX_EVENTS);
    assert(length <= sizeof(events[0].name));
    event = &events[event_count++];
    memset(event, 0, sizeof(*event));
    event->operation = operation;
    event->base = base;
    event->access = access;
    event->length = length;
    if (length != 0)
        memcpy(event->name, name, length);
}

static void reset_events(void)
{
    memset(events, 0, sizeof(events));
    event_count = 0;
}

static void *packet_allocate(void *context, size_t size)
{
    (void)context;
    if (fail_allocations != 0)
    {
        fail_allocations--;
        return NULL;
    }
    return calloc(1, size);
}

static void packet_free(void *context, void *allocation, size_t size)
{
    (void)context;
    (void)size;
    free(allocation);
}

static int32_t packet_now(void *context, int64_t *seconds,
    uint32_t *nanoseconds)
{
    (void)context;
    *seconds = INT64_C(252547261);
    *nanoseconds = UINT32_C(40000000);
    return 0;
}

static void initialize_packet(struct DosPacket *packet, LONG action)
{
    memset(packet, 0, sizeof(*packet));
    packet->dp_Type = action;
}

static SIPTR packet_bstr(const char *text)
{
    return (SIPTR)MKBADDR((void *)text);
}

int32_t afsplus_aros_locate(struct AfsplusAros *filesystem,
    uint64_t base_lock, const uint8_t *name, uint32_t name_length,
    uint32_t access, uint64_t *output_lock)
{
    assert(filesystem == STUB_FILESYSTEM);
    record('L', base_lock, name, name_length, access);
    *output_lock = next_lock++;
    return 0;
}

int32_t afsplus_aros_duplicate_lock(struct AfsplusAros *filesystem,
    uint64_t lock, uint64_t *output_lock)
{
    assert(filesystem == STUB_FILESYSTEM);
    record('D', lock, NULL, 0, 0);
    *output_lock = next_lock++;
    return 0;
}

int32_t afsplus_aros_parent_lock(struct AfsplusAros *filesystem,
    uint64_t lock, uint64_t *output_lock)
{
    assert(filesystem == STUB_FILESYSTEM);
    record('P', lock, NULL, 0, AFSPLUS_AROS_LOCK_SHARED);
    *output_lock = next_lock++;
    return 0;
}

int32_t afsplus_aros_parent_lock_with_access(struct AfsplusAros *filesystem,
    uint64_t lock, uint32_t access, uint64_t *output_lock)
{
    assert(filesystem == STUB_FILESYSTEM);
    record('P', lock, NULL, 0, access);
    *output_lock = next_lock++;
    return 0;
}

int32_t afsplus_aros_same_lock(struct AfsplusAros *filesystem,
    uint64_t first_lock, uint64_t second_lock, uint32_t *output_same)
{
    assert(filesystem == STUB_FILESYSTEM);
    *output_same = first_lock == second_lock;
    return 0;
}

int32_t afsplus_aros_free_lock(struct AfsplusAros *filesystem, uint64_t lock)
{
    assert(filesystem == STUB_FILESYSTEM);
    record('F', lock, NULL, 0, 0);
    return 0;
}

int32_t afsplus_aros_open(struct AfsplusAros *filesystem,
    uint64_t base_lock, const uint8_t *name, uint32_t name_length,
    uint32_t mode, int64_t now_seconds, uint32_t now_nanoseconds,
    uint64_t *output_file)
{
    assert(filesystem == STUB_FILESYSTEM);
    assert(now_seconds == INT64_C(252547261));
    assert(now_nanoseconds == UINT32_C(40000000));
    record('O', base_lock, name, name_length, mode);
    *output_file = next_file++;
    file_position = 0;
    file_size = 0;
    return 0;
}

int32_t afsplus_aros_parent_of_file(struct AfsplusAros *filesystem,
    uint64_t file, uint64_t *output_lock)
{
    assert(filesystem == STUB_FILESYSTEM);
    record('Q', file, NULL, 0, 0);
    *output_lock = next_lock++;
    return 0;
}

int32_t afsplus_aros_lock_from_file(struct AfsplusAros *filesystem,
    uint64_t file, uint64_t *output_lock)
{
    assert(filesystem == STUB_FILESYSTEM);
    record('K', file, NULL, 0, 0);
    *output_lock = next_lock++;
    return 0;
}

int32_t afsplus_aros_close(struct AfsplusAros *filesystem, uint64_t file)
{
    assert(filesystem == STUB_FILESYSTEM);
    assert(file >= 100);
    close_count++;
    return 0;
}

int32_t afsplus_aros_read(struct AfsplusAros *filesystem, uint64_t file,
    uint8_t *destination, uint32_t length, uint32_t *output_count)
{
    static const uint8_t contents[] = "hello";
    uint32_t remaining;
    uint32_t count;

    assert(filesystem == STUB_FILESYSTEM);
    assert(file >= 100);
    remaining = file_position < sizeof(contents) - 1
        ? (uint32_t)(sizeof(contents) - 1 - file_position) : 0;
    count = length < remaining ? length : remaining;
    if (count != 0)
        memcpy(destination, contents + file_position, count);
    file_position += count;
    *output_count = count;
    return 0;
}

int32_t afsplus_aros_write(struct AfsplusAros *filesystem, uint64_t file,
    const uint8_t *source, uint32_t length, int64_t now_seconds,
    uint32_t now_nanoseconds, uint32_t *output_count)
{
    assert(filesystem == STUB_FILESYSTEM);
    assert(file >= 100);
    assert(source != NULL || length == 0);
    assert(now_seconds == INT64_C(252547261));
    assert(now_nanoseconds == UINT32_C(40000000));
    file_position += length;
    if (file_size < file_position)
        file_size = file_position;
    *output_count = length;
    return 0;
}

int32_t afsplus_aros_seek(struct AfsplusAros *filesystem, uint64_t file,
    int64_t offset, uint32_t mode, uint64_t *output_old_position)
{
    uint64_t base;

    assert(filesystem == STUB_FILESYSTEM);
    assert(file >= 100);
    *output_old_position = file_position;
    base = mode == AFSPLUS_AROS_SEEK_BEGINNING ? 0
        : mode == AFSPLUS_AROS_SEEK_CURRENT ? file_position : file_size;
    assert(offset >= 0 || (uint64_t)(-offset) <= base);
    file_position = offset < 0 ? base - (uint64_t)(-offset)
        : base + (uint64_t)offset;
    return 0;
}

int32_t afsplus_aros_file_position(struct AfsplusAros *filesystem,
    uint64_t file, uint64_t *output_position)
{
    assert(filesystem == STUB_FILESYSTEM);
    assert(file >= 100);
    *output_position = file_position;
    return 0;
}

int32_t afsplus_aros_file_size(struct AfsplusAros *filesystem,
    uint64_t file, uint64_t *output_size)
{
    assert(filesystem == STUB_FILESYSTEM);
    assert(file >= 100);
    *output_size = file_size;
    return 0;
}

int32_t afsplus_aros_set_file_size(struct AfsplusAros *filesystem,
    uint64_t file, int64_t offset, uint32_t mode, int64_t now_seconds,
    uint32_t now_nanoseconds, uint64_t *output_size)
{
    uint64_t base;

    assert(filesystem == STUB_FILESYSTEM);
    assert(file >= 100);
    assert(now_seconds == INT64_C(252547261));
    assert(now_nanoseconds == UINT32_C(40000000));
    base = mode == AFSPLUS_AROS_SEEK_BEGINNING ? 0
        : mode == AFSPLUS_AROS_SEEK_CURRENT ? file_position : file_size;
    file_size = offset < 0 ? base - (uint64_t)(-offset)
        : base + (uint64_t)offset;
    *output_size = file_size;
    return 0;
}

int32_t afsplus_aros_fsync(struct AfsplusAros *filesystem, uint64_t file)
{
    assert(filesystem == STUB_FILESYSTEM);
    assert(file >= 100);
    fsync_count++;
    return 0;
}

int32_t afsplus_aros_flush(struct AfsplusAros *filesystem)
{
    assert(filesystem == STUB_FILESYSTEM);
    flush_count++;
    return 0;
}

int32_t afsplus_aros_create_directory(struct AfsplusAros *filesystem,
    uint64_t base_lock, const uint8_t *name, uint32_t name_length,
    int64_t now_seconds, uint32_t now_nanoseconds, uint64_t *output_lock)
{
    assert(filesystem == STUB_FILESYSTEM);
    assert(now_seconds == INT64_C(252547261));
    assert(now_nanoseconds == UINT32_C(40000000));
    record('C', base_lock, name, name_length, 0);
    *output_lock = next_lock++;
    created_lock_id = *output_lock;
    return 0;
}

int32_t afsplus_aros_delete_object(struct AfsplusAros *filesystem,
    uint64_t base_lock, const uint8_t *name, uint32_t name_length,
    int64_t now_seconds, uint32_t now_nanoseconds)
{
    assert(filesystem == STUB_FILESYSTEM);
    assert(now_seconds == INT64_C(252547261));
    assert(now_nanoseconds == UINT32_C(40000000));
    record('X', base_lock, name, name_length, 0);
    return 0;
}

int32_t afsplus_aros_rename(struct AfsplusAros *filesystem,
    uint64_t source_base_lock, const uint8_t *source_name,
    uint32_t source_name_length, uint64_t target_base_lock,
    const uint8_t *target_name, uint32_t target_name_length,
    int64_t now_seconds, uint32_t now_nanoseconds)
{
    assert(filesystem == STUB_FILESYSTEM);
    assert(now_seconds == INT64_C(252547261));
    assert(now_nanoseconds == UINT32_C(40000000));
    record('R', source_base_lock, source_name, source_name_length, 0);
    record('r', target_base_lock, target_name, target_name_length, 0);
    return 0;
}

int32_t afsplus_aros_make_hard_link(struct AfsplusAros *filesystem,
    uint64_t target_base_lock, const uint8_t *target_name,
    uint32_t target_name_length, uint64_t source_lock,
    int64_t now_seconds, uint32_t now_nanoseconds)
{
    assert(filesystem == STUB_FILESYSTEM);
    assert(now_seconds == INT64_C(252547261));
    assert(now_nanoseconds == UINT32_C(40000000));
    record('H', target_base_lock, target_name, target_name_length,
        (uint32_t)source_lock);
    return 0;
}

static int32_t examine_common(struct AfsplusArosFileInfo *output,
    uint8_t *name, uint32_t name_capacity)
{
    static const uint8_t filename[] = "entry";

    assert(name_capacity >= sizeof(filename) - 1);
    examine_count++;
    memset(output, 0, sizeof(*output));
    memcpy(name, filename, sizeof(filename) - 1);
    output->disk_key = UINT64_C(0x12345678);
    output->size = UINT64_C(0x100000002);
    output->blocks = UINT64_C(0x100001);
    output->modified_seconds = INT64_C(252547261);
    output->modified_nanoseconds = UINT32_C(40000000);
    output->directory_entry_type = ST_FILE;
    output->entry_type = ST_FILE;
    output->name_length = sizeof(filename) - 1;
    return 0;
}

int32_t afsplus_aros_examine_lock(struct AfsplusAros *filesystem,
    uint64_t lock, struct AfsplusArosFileInfo *output,
    uint8_t *name, uint32_t name_capacity)
{
    assert(filesystem == STUB_FILESYSTEM);
    assert(lock != 0);
    return examine_common(output, name, name_capacity);
}

int32_t afsplus_aros_examine_file(struct AfsplusAros *filesystem,
    uint64_t file, struct AfsplusArosFileInfo *output,
    uint8_t *name, uint32_t name_capacity)
{
    assert(filesystem == STUB_FILESYSTEM);
    assert(file >= 100);
    return examine_common(output, name, name_capacity);
}

int32_t afsplus_aros_examine_next(struct AfsplusAros *filesystem,
    uint64_t lock, struct AfsplusArosFileInfo *output,
    uint8_t *name, uint32_t name_capacity)
{
    assert(filesystem == STUB_FILESYSTEM);
    assert(lock != 0);
    if (stub_directory_size >= 0)
    {
        int32_t error;

        if (stub_directory_at >= stub_directory_size)
            return ERROR_NO_MORE_ENTRIES;
        if (stub_directory_at == stub_directory_fail_at)
        {
            stub_directory_fail_at = -1;
            return ERROR_SEEK_ERROR;
        }
        error = examine_common(output, name, name_capacity);
        name[0] = 'e';
        name[1] = (uint8_t)('0' + stub_directory_at);
        output->name_length = 2;
        /* ExNext reports fib_DirEntryType from directory_entry_type, and
         * ExAll must agree with it; the two fields differ on purpose. */
        output->directory_entry_type = ST_LINKFILE;
        output->entry_type = ST_FILE;
        output->size = (uint64_t)stub_directory_at + 10;
        output->protection = UINT32_C(0x40) + (uint32_t)stub_directory_at;
        stub_directory_at++;
        return error;
    }
    return examine_common(output, name, name_capacity);
}

int32_t afsplus_aros_rewind_directory(struct AfsplusAros *filesystem,
    uint64_t lock)
{
    assert(filesystem == STUB_FILESYSTEM);
    assert(lock != 0);
    rewind_count++;
    stub_directory_at = 0;
    return 0;
}

int32_t afsplus_aros_disk_info(struct AfsplusAros *filesystem,
    struct AfsplusArosDiskInfo *output)
{
    assert(filesystem == STUB_FILESYSTEM);
    memset(output, 0, sizeof(*output));
    output->total_blocks = UINT64_C(0x100000005);
    output->used_blocks = 12;
    output->bytes_per_block = 4096;
    output->disk_type = (int32_t)UINT32_C(0x4146532b);
    output->in_use = 1;
    return 0;
}

int32_t afsplus_aros_interface(struct AfsplusArosInterface *output)
{
    assert(output->struct_size == sizeof(*output));
    output->abi_version = AFSPLUS_AROS_ABI_VERSION;
    output->interface_revision = stub_revision;
    output->groups = stub_groups;
    return 0;
}

/* The comment the fake filesystem stores for every object. */
static uint8_t stub_comment[256];
static uint32_t stub_comment_length;
static int32_t stub_comment_error;

static int32_t comment_common(uint8_t *comment, uint32_t capacity,
    uint32_t *output_length)
{
    uint32_t length = stub_comment_length < capacity
        ? stub_comment_length : capacity;

    if (stub_comment_error != 0)
        return stub_comment_error;
    /* The packet layer always offers exactly the width of fib_Comment. */
    assert(capacity == 79);
    memcpy(comment, stub_comment, length);
    *output_length = length;
    return 0;
}

int32_t afsplus_aros_comment(struct AfsplusAros *filesystem,
    uint64_t base_lock, const uint8_t *name, uint32_t name_length,
    uint8_t *comment, uint32_t comment_capacity, uint32_t *output_length)
{
    assert(filesystem == STUB_FILESYSTEM);
    assert(base_lock != 0);
    record('c', base_lock, name, name_length, 0);
    return comment_common(comment, comment_capacity, output_length);
}

int32_t afsplus_aros_file_comment(struct AfsplusAros *filesystem,
    uint64_t file, uint8_t *comment, uint32_t comment_capacity,
    uint32_t *output_length)
{
    assert(filesystem == STUB_FILESYSTEM);
    assert(file >= 100);
    record('c', file, NULL, 0, 1);
    return comment_common(comment, comment_capacity, output_length);
}

int32_t afsplus_aros_set_comment(struct AfsplusAros *filesystem,
    uint64_t base_lock, const uint8_t *name, uint32_t name_length,
    const uint8_t *comment, uint32_t comment_length,
    int64_t now_seconds, uint32_t now_nanoseconds)
{
    assert(filesystem == STUB_FILESYSTEM);
    assert(now_seconds == INT64_C(252547261));
    assert(now_nanoseconds == UINT32_C(40000000));
    assert(comment_length <= 79);
    record('C', base_lock, name, name_length, 0);
    memcpy(stub_comment, comment, comment_length);
    stub_comment_length = comment_length;
    return 0;
}

int32_t afsplus_aros_set_protection(struct AfsplusAros *filesystem,
    uint64_t base_lock, const uint8_t *name, uint32_t name_length,
    uint32_t protection, int64_t now_seconds, uint32_t now_nanoseconds)
{
    assert(filesystem == STUB_FILESYSTEM);
    assert(now_seconds == INT64_C(252547261));
    assert(now_nanoseconds == UINT32_C(40000000));
    record('p', base_lock, name, name_length, 0);
    stub_protection = protection;
    return 0;
}

int32_t afsplus_aros_set_modified(struct AfsplusAros *filesystem,
    uint64_t base_lock, const uint8_t *name, uint32_t name_length,
    int64_t modified_seconds, uint32_t modified_nanoseconds,
    int64_t now_seconds, uint32_t now_nanoseconds)
{
    assert(filesystem == STUB_FILESYSTEM);
    assert(now_seconds == INT64_C(252547261));
    assert(now_nanoseconds == UINT32_C(40000000));
    record('d', base_lock, name, name_length, 0);
    stub_modified_seconds = modified_seconds;
    stub_modified_nanoseconds = modified_nanoseconds;
    return 0;
}

int32_t afsplus_aros_make_soft_link(struct AfsplusAros *filesystem,
    uint64_t base_lock, const uint8_t *name, uint32_t name_length,
    const uint8_t *target, uint32_t target_length,
    int64_t now_seconds, uint32_t now_nanoseconds)
{
    assert(filesystem == STUB_FILESYSTEM);
    assert(now_seconds == INT64_C(252547261));
    assert(now_nanoseconds == UINT32_C(40000000));
    record('S', base_lock, name, name_length, 0);
    record('s', 0, target, target_length, 0);
    return 0;
}

/* Only the component "link" is a soft link in the stub namespace. */
int32_t afsplus_aros_read_soft_link(struct AfsplusAros *filesystem,
    uint64_t base_lock, const uint8_t *name, uint32_t name_length,
    uint8_t *target, uint32_t target_capacity, uint32_t *output_required)
{
    uint32_t length = (uint32_t)strlen(stub_link_target);

    assert(filesystem == STUB_FILESYSTEM);
    (void)base_lock;
    if (name_length != 4 || memcmp(name, "link", 4) != 0)
        return ERROR_OBJECT_WRONG_TYPE;
    if (length <= target_capacity)
        memcpy(target, stub_link_target, length);
    *output_required = length;
    return 0;
}

int32_t afsplus_aros_watch_add(struct AfsplusAros *filesystem,
    uint64_t base_lock, const uint8_t *name, uint32_t name_length,
    uint64_t *output_watch)
{
    assert(filesystem == STUB_FILESYSTEM);
    record('W', base_lock, name, name_length, 0);
    *output_watch = stub_next_watch++;
    return 0;
}

int32_t afsplus_aros_watch_remove(struct AfsplusAros *filesystem,
    uint64_t watch)
{
    assert(filesystem == STUB_FILESYSTEM);
    assert(watch >= 500);
    stub_removed_watches++;
    return 0;
}

int32_t afsplus_aros_watch_drain(struct AfsplusAros *filesystem,
    uint64_t *watches, uint32_t capacity, uint32_t *output_count)
{
    uint32_t i;

    assert(filesystem == STUB_FILESYSTEM);
    assert(capacity >= stub_fired_count);
    for (i = 0; i < stub_fired_count; i++)
        watches[i] = stub_fired[i];
    *output_count = stub_fired_count;
    stub_fired_count = 0;
    return 0;
}

int32_t afsplus_aros_set_volume_label(struct AfsplusAros *filesystem,
    const uint8_t *label, uint32_t label_length, int64_t now_seconds,
    uint32_t now_nanoseconds)
{
    assert(filesystem == STUB_FILESYSTEM);
    assert(now_seconds == INT64_C(252547261));
    assert(now_nanoseconds == UINT32_C(40000000));
    record('V', 0, label, label_length, 0);
    return stub_label_error;
}

/* Fakes of the 64-bit groups the extension packet reaches. Each records the
 * object it was given and echoes its scalar arguments into ext_seen. */
static uint64_t ext_seen[5];
static int32_t ext_error;

static int32_t ext_call(char operation, uint64_t object, const uint8_t *name,
    uint32_t name_length, uint64_t a, uint64_t b, uint64_t c, uint64_t d)
{
    record(operation, object, name, name_length, 0);
    ext_seen[0] = a;
    ext_seen[1] = b;
    ext_seen[2] = c;
    ext_seen[3] = d;
    return ext_error;
}

/* The application's report buffer, which no entry point may be handed. */
static uint8_t *ext_caller_buffer;

int32_t afsplus_aros_capabilities(struct AfsplusAros *filesystem,
    struct AfsplusArosCapabilities *output)
{
    assert(filesystem == STUB_FILESYSTEM);
    assert((uint8_t *)output != ext_caller_buffer);
    /* Another task enlarges the declared size while the packet is here. */
    if (ext_caller_buffer != NULL)
    {
        uint32_t huge = UINT32_C(0xFFFFFFF0);

        memcpy(ext_caller_buffer, &huge, sizeof(huge));
    }
    /* The entry point's own floor, as afsplus_aros.h states it. */
    if (output->struct_size < AFSPLUS_AROS_CAPABILITIES_FIRST_LAYOUT)
        return ERROR_BAD_NUMBER;
    /* It fills what it knows and stores that size, never the caller's. */
    output->struct_size = sizeof(*output);
    output->mount_mode = 77;
    return ext_call('1', 0, NULL, 0, output->struct_size, 0, 0, 0);
}

int32_t afsplus_aros_read_at(struct AfsplusAros *filesystem, uint64_t file,
    uint64_t offset, uint8_t *destination, uint32_t length,
    uint32_t *output_count)
{
    assert(filesystem == STUB_FILESYSTEM);
    if (ext_error == 0 && length >= 2)
    {
        destination[0] = 'o';
        destination[1] = 'k';
        *output_count = 2;
    }
    return ext_call('2', file, NULL, 0, offset, length, 0, 0);
}

int32_t afsplus_aros_write_at(struct AfsplusAros *filesystem, uint64_t file,
    uint64_t offset, const uint8_t *source, uint32_t length,
    int64_t now_seconds, uint32_t now_nanoseconds, uint32_t *output_count)
{
    assert(filesystem == STUB_FILESYSTEM);
    assert(now_seconds == INT64_C(252547261));
    assert(now_nanoseconds == UINT32_C(40000000));
    *output_count = length;
    return ext_call('3', file, source, length, offset, length, 0, 0);
}

int32_t afsplus_aros_clone_file(struct AfsplusAros *filesystem,
    uint64_t source_lock, uint64_t target_base_lock,
    const uint8_t *target_name, uint32_t target_name_length,
    int64_t now_seconds, uint32_t now_nanoseconds)
{
    assert(filesystem == STUB_FILESYSTEM);
    assert(now_seconds == INT64_C(252547261));
    (void)now_nanoseconds;
    return ext_call('4', source_lock, target_name, target_name_length,
        target_base_lock, 0, 0, 0);
}

int32_t afsplus_aros_clone_range(struct AfsplusAros *filesystem,
    uint64_t source_file, uint64_t source_offset, uint64_t target_file,
    uint64_t target_offset, uint64_t length, int64_t now_seconds,
    uint32_t now_nanoseconds)
{
    assert(filesystem == STUB_FILESYSTEM);
    assert(now_seconds == INT64_C(252547261));
    (void)now_nanoseconds;
    return ext_call('5', source_file, NULL, 0, source_offset, target_file,
        target_offset, length);
}

int32_t afsplus_aros_preallocate(struct AfsplusAros *filesystem,
    uint64_t file, uint64_t offset, uint64_t length, int64_t now_seconds,
    uint32_t now_nanoseconds)
{
    assert(filesystem == STUB_FILESYSTEM);
    assert(now_seconds == INT64_C(252547261));
    (void)now_nanoseconds;
    return ext_call('6', file, NULL, 0, offset, length, 0, 0);
}

int32_t afsplus_aros_replace(struct AfsplusAros *filesystem,
    uint64_t source_base_lock, const uint8_t *source_name,
    uint32_t source_name_length, uint64_t target_base_lock,
    const uint8_t *target_name, uint32_t target_name_length,
    int64_t now_seconds, uint32_t now_nanoseconds)
{
    assert(filesystem == STUB_FILESYSTEM);
    assert(now_seconds == INT64_C(252547261));
    (void)now_nanoseconds;
    ext_seen[4] = target_name_length == 3
        && memcmp(target_name, "new", 3) == 0;
    return ext_call('7', source_base_lock, source_name, source_name_length,
        target_base_lock, 0, 0, 0);
}

int32_t afsplus_aros_advise(struct AfsplusAros *filesystem, uint64_t file,
    uint64_t offset, uint64_t length, uint32_t hint, uint32_t *output_effect)
{
    assert(filesystem == STUB_FILESYSTEM);
    *output_effect = 9;
    return ext_call('8', file, NULL, 0, offset, length, hint, 0);
}

int32_t afsplus_aros_info_json(struct AfsplusAros *filesystem,
    uint8_t *buffer, uint32_t capacity, uint32_t *output_required)
{
    assert(filesystem == STUB_FILESYSTEM);
    *output_required = 4;
    if (capacity >= 4)
        memcpy(buffer, "{\"a\"", 4);
    return ext_call('9', 0, NULL, 0, capacity, 0, 0, 0);
}

int32_t afsplus_aros_counters(struct AfsplusAros *filesystem,
    struct AfsplusArosCounters *output)
{
    assert(filesystem == STUB_FILESYSTEM);
    return ext_call('a', 0, NULL, 0, output->struct_size, 0, 0, 0);
}

int32_t afsplus_aros_health(struct AfsplusAros *filesystem,
    struct AfsplusArosHealth *output)
{
    assert(filesystem == STUB_FILESYSTEM);
    return ext_call('b', 0, NULL, 0, output->struct_size, 0, 0, 0);
}

int32_t afsplus_aros_extent_map(struct AfsplusAros *filesystem,
    uint64_t file, uint64_t offset, uint64_t length,
    struct AfsplusArosExtent *extents, uint32_t capacity,
    uint32_t *output_count, uint32_t *output_complete,
    uint64_t *output_next_offset)
{
    assert(filesystem == STUB_FILESYSTEM);
    (void)extents;
    /* The entry point's own range, as afsplus_aros.h states it. */
    if (capacity == 0 || capacity > 64)
        return ERROR_BAD_NUMBER;
    *output_count = capacity;
    *output_complete = 1;
    *output_next_offset = offset + length;
    return ext_call('c', file, NULL, 0, offset, length, capacity, 0);
}

int32_t afsplus_aros_lookup_id(struct AfsplusAros *filesystem,
    uint64_t base_lock, const uint8_t *name, uint32_t name_length,
    uint64_t *output_object_id)
{
    assert(filesystem == STUB_FILESYSTEM);
    *output_object_id = UINT64_C(0x1122334455);
    return ext_call('d', base_lock, name, name_length, 0, 0, 0, 0);
}

int32_t afsplus_aros_stat_id(struct AfsplusAros *filesystem,
    uint64_t object_id, struct AfsplusArosStat *output)
{
    assert(filesystem == STUB_FILESYSTEM);
    return ext_call('e', 0, NULL, 0, object_id, output->struct_size, 0, 0);
}

/* Host memory with an inaccessible page behind it, declared here because the
 * matrix compiles against the AROS C library headers. */
void *mmap(void *address, size_t length, int protection, int flags, int fd,
    long long offset);
int mprotect(void *address, size_t length, int protection);
#ifdef __APPLE__
#define STUB_MAP_ANON_PRIVATE (0x1000 | 0x0002)
#else
#define STUB_MAP_ANON_PRIVATE (0x20 | 0x02)
#endif
#define STUB_GUARD_PAGE 65536

/* The last `size` readable bytes before a page that faults on any access. */
static uint8_t *before_guard_page(size_t size)
{
    uint8_t *region = mmap(NULL, 2 * STUB_GUARD_PAGE, 3,
        STUB_MAP_ANON_PRIVATE, -1, 0);

    assert(region != (uint8_t *)-1 && region != NULL);
    assert(mprotect(region + STUB_GUARD_PAGE, STUB_GUARD_PAGE, 0) == 0);
    return region + STUB_GUARD_PAGE - size;
}

static struct DosPacket *completed[24];
static size_t completed_count;

static void packet_complete(void *context, struct DosPacket *packet)
{
    (void)context;
    assert(completed_count < sizeof(completed) / sizeof(completed[0]));
    completed[completed_count++] = packet;
}

static int32_t packet_relabel(void *context, uint32_t phase,
    const uint8_t *name, uint32_t name_length)
{
    (void)context;
    assert(relabel_calls < sizeof(relabel_phases) - 1);
    relabel_phases[relabel_calls++] = "PCA"[phase];
    if (phase == AFSPLUS_AROS_RELABEL_PREPARE)
        return stub_relabel_prepare_error;
    if (phase == AFSPLUS_AROS_RELABEL_COMMIT)
    {
        assert(name_length <= sizeof(relabelled));
        memcpy(relabelled, name, name_length);
        relabelled_length = name_length;
    }
    return 0;
}

static void packet_notify(void *context, struct NotifyRequest *request)
{
    (void)context;
    assert(delivered_count < 8);
    delivered[delivered_count++] = request;
}

int32_t afsplus_aros_open_from_lock(struct AfsplusAros *filesystem,
    uint64_t lock, uint64_t *output_file)
{
    assert(filesystem == STUB_FILESYSTEM);
    record('O', lock, NULL, 0, 0);
    if (stub_open_from_lock_error != 0)
        return stub_open_from_lock_error;
    *output_file = next_file++;
    return 0;
}

int32_t afsplus_aros_change_lock_mode(struct AfsplusAros *filesystem,
    uint64_t lock, uint32_t access)
{
    assert(filesystem == STUB_FILESYSTEM);
    record('m', lock, NULL, 0, access);
    stub_changed_access = access;
    return stub_change_mode_error;
}

int32_t afsplus_aros_change_file_mode(struct AfsplusAros *filesystem,
    uint64_t file, uint32_t access)
{
    assert(filesystem == STUB_FILESYSTEM);
    record('M', file, NULL, 0, access);
    stub_changed_access = access;
    return stub_change_mode_error;
}

int32_t afsplus_aros_set_write_protect(struct AfsplusAros *filesystem,
    uint32_t protect, uint32_t key)
{
    assert(filesystem == STUB_FILESYSTEM);
    record('w', 0, NULL, 0, protect);
    if (!protect && key != stub_protect_key)
        return ERROR_DISK_WRITE_PROTECTED;
    stub_protect = protect;
    stub_protect_key = key;
    return 0;
}

int32_t afsplus_aros_lock_record(struct AfsplusAros *filesystem,
    uint64_t file, uint64_t offset, uint64_t length, uint32_t exclusive)
{
    assert(filesystem == STUB_FILESYSTEM);
    record('k', file, NULL, 0, exclusive);
    stub_record_offset = offset;
    stub_record_length = length;
    stub_record_exclusive = exclusive;
    return stub_record_error;
}

int32_t afsplus_aros_free_record(struct AfsplusAros *filesystem,
    uint64_t file, uint64_t offset, uint64_t length)
{
    assert(filesystem == STUB_FILESYSTEM);
    record('u', file, NULL, 0, 0);
    stub_record_offset = offset;
    stub_record_length = length;
    return stub_free_succeeds ? 0 : stub_record_error;
}

static void assert_event(size_t index, char operation, const char *name,
    uint32_t access)
{
    size_t length = strlen(name);

    assert(index < event_count);
    assert(events[index].operation == operation);
    assert(events[index].length == length);
    assert(memcmp(events[index].name, name, length) == 0);
    assert(events[index].access == access);
}

int main(void)
{
    struct AfsplusArosPacketConfig config;
    struct AfsplusArosPacketContext *context = NULL;
    struct DosPacket packet;
    struct FileHandle public_file;
    struct FileInfoBlock fib;
    struct FileInfoBlock64 fib64;
    struct InfoData disk;
    struct InfoData64 disk64;
    BPTR root;
    BPTR located;
    BPTR created;
    uint8_t write_data[4] = { 1, 2, 3, 4 };
    uint8_t read_data[5];

    memset(&config, 0, sizeof(config));
    config.abi_version = AFSPLUS_AROS_PACKET_ABI_VERSION;
    config.struct_size = sizeof(config);
    config.filesystem = STUB_FILESYSTEM;
    config.handler_port = (struct MsgPort *)(uintptr_t)UINT32_C(0x2000);
    config.volume_node = MKBADDR((void *)(uintptr_t)UINT32_C(0x3000));
    config.allocate = packet_allocate;
    config.free = packet_free;
    config.now = packet_now;
    config.notify = packet_notify;
    config.relabel = packet_relabel;
    config.complete = packet_complete;
    assert(afsplus_aros_packet_create(&config, &context) == 0);
    assert(context != NULL);

    initialize_packet(&packet, ACTION_LOCATE_OBJECT);
    packet.dp_Arg2 = packet_bstr("");
    packet.dp_Arg3 = SHARED_LOCK;
    assert(afsplus_aros_packet_process(context, &packet) == 0);
    assert(packet.dp_Res2 == 0 && packet.dp_Res1 != 0);
    root = (BPTR)packet.dp_Res1;

    reset_events();
    initialize_packet(&packet, ACTION_LOCATE_OBJECT);
    packet.dp_Arg1 = (SIPTR)root;
    packet.dp_Arg2 = packet_bstr("AFS+:one//two");
    packet.dp_Arg3 = EXCLUSIVE_LOCK;
    assert(afsplus_aros_packet_process(context, &packet) == 0);
    assert(packet.dp_Res2 == 0 && packet.dp_Res1 != 0);
    located = (BPTR)packet.dp_Res1;
    assert_event(0, 'L', "one", AFSPLUS_AROS_LOCK_SHARED);
    assert(events[0].base == 0);
    assert(events[1].operation == 'P');
    assert(events[1].access == AFSPLUS_AROS_LOCK_SHARED);
    assert(events[2].operation == 'F');
    assert_event(3, 'L', "two", AFSPLUS_AROS_LOCK_EXCLUSIVE);
    assert(events[4].operation == 'F');

    reset_events();
    fail_allocations = 1;
    memset(&public_file, 0, sizeof(public_file));
    initialize_packet(&packet, ACTION_FINDOUTPUT);
    packet.dp_Arg1 = (SIPTR)MKBADDR(&public_file);
    packet.dp_Arg2 = (SIPTR)root;
    packet.dp_Arg3 = packet_bstr("must-not-exist");
    assert(afsplus_aros_packet_process(context, &packet) == 0);
    assert(packet.dp_Res1 == DOSFALSE && packet.dp_Res2 == ERROR_NO_FREE_STORE);
    assert(event_count == 0);

    fail_allocations = 1;
    initialize_packet(&packet, ACTION_CREATE_DIR);
    packet.dp_Arg1 = (SIPTR)root;
    packet.dp_Arg2 = packet_bstr("must-not-be-created");
    assert(afsplus_aros_packet_process(context, &packet) == 0);
    assert(packet.dp_Res1 == DOSFALSE && packet.dp_Res2 == ERROR_NO_FREE_STORE);
    assert(event_count == 0);

    reset_events();
    initialize_packet(&packet, ACTION_CREATE_DIR);
    packet.dp_Arg1 = (SIPTR)root;
    packet.dp_Arg2 = packet_bstr("parent/newdir");
    assert(afsplus_aros_packet_process(context, &packet) == 0);
    assert(packet.dp_Res1 != 0 && packet.dp_Res2 == 0);
    created = (BPTR)packet.dp_Res1;
    assert_event(0, 'L', "parent", AFSPLUS_AROS_LOCK_SHARED);
    assert_event(1, 'C', "newdir", 0);
    assert(events[2].operation == 'F');

    reset_events();
    initialize_packet(&packet, ACTION_RENAME_OBJECT);
    packet.dp_Arg1 = (SIPTR)root;
    packet.dp_Arg2 = packet_bstr("old");
    packet.dp_Arg3 = (SIPTR)root;
    packet.dp_Arg4 = packet_bstr("parent/new");
    assert(afsplus_aros_packet_process(context, &packet) == 0);
    assert(packet.dp_Res1 == DOSTRUE && packet.dp_Res2 == 0);
    assert_event(0, 'L', "parent", AFSPLUS_AROS_LOCK_SHARED);
    assert_event(1, 'R', "old", 0);
    assert_event(2, 'r', "new", 0);
    assert(events[3].operation == 'F');

    reset_events();
    initialize_packet(&packet, ACTION_MAKE_LINK);
    packet.dp_Arg1 = (SIPTR)root;
    packet.dp_Arg2 = packet_bstr("links/copy");
    packet.dp_Arg3 = (SIPTR)created;
    packet.dp_Arg4 = LINK_HARD;
    assert(afsplus_aros_packet_process(context, &packet) == 0);
    assert(packet.dp_Res1 == DOSTRUE && packet.dp_Res2 == 0);
    assert_event(0, 'L', "links", AFSPLUS_AROS_LOCK_SHARED);
    assert_event(1, 'H', "copy", (uint32_t)created_lock_id);
    assert(events[2].operation == 'F');

    reset_events();
    initialize_packet(&packet, ACTION_DELETE_OBJECT);
    packet.dp_Arg1 = (SIPTR)root;
    packet.dp_Arg2 = packet_bstr("trash/dead");
    assert(afsplus_aros_packet_process(context, &packet) == 0);
    assert(packet.dp_Res1 == DOSTRUE && packet.dp_Res2 == 0);
    assert_event(0, 'L', "trash", AFSPLUS_AROS_LOCK_SHARED);
    assert_event(1, 'X', "dead", 0);
    assert(events[2].operation == 'F');

    reset_events();
    memset(&public_file, 0, sizeof(public_file));
    initialize_packet(&packet, ACTION_FINDOUTPUT);
    packet.dp_Arg1 = (SIPTR)MKBADDR(&public_file);
    packet.dp_Arg2 = (SIPTR)root;
    packet.dp_Arg3 = packet_bstr("dir/file");
    assert(afsplus_aros_packet_process(context, &packet) == 0);
    assert(packet.dp_Res1 == DOSTRUE && packet.dp_Res2 == 0);
    assert(public_file.fh_Arg1 != 0);
    assert_event(0, 'L', "dir", AFSPLUS_AROS_LOCK_SHARED);
    assert_event(1, 'O', "file", AFSPLUS_AROS_OPEN_NEW_FILE);
    assert(events[2].operation == 'F');

    initialize_packet(&packet, ACTION_EXAMINE_FH);
    packet.dp_Arg1 = public_file.fh_Arg1;
    packet.dp_Arg2 = 0;
    assert(afsplus_aros_packet_process(context, &packet) == 0);
    assert(packet.dp_Res1 == DOSFALSE && packet.dp_Res2 == ERROR_INVALID_LOCK);
    assert(examine_count == 0);

    initialize_packet(&packet, ACTION_WRITE);
    packet.dp_Arg1 = public_file.fh_Arg1;
    packet.dp_Arg2 = (SIPTR)write_data;
    packet.dp_Arg3 = sizeof(write_data);
    assert(afsplus_aros_packet_process(context, &packet) == 0);
    assert(packet.dp_Res1 == (SIPTR)sizeof(write_data));

    initialize_packet(&packet, ACTION_SEEK64);
    packet.dp_Arg1 = public_file.fh_Arg1;
    packet.dp_Arg2 = INT64_C(8589934592);
    packet.dp_Arg3 = OFFSET_BEGINNING;
    assert(afsplus_aros_packet_process(context, &packet) == 0);
    assert(packet.dp_Res1 == 4 && packet.dp_Res2 == 0);

    initialize_packet(&packet, ACTION_SET_FILE_SIZE64);
    packet.dp_Arg1 = public_file.fh_Arg1;
    packet.dp_Arg2 = INT64_C(4294967301);
    packet.dp_Arg3 = OFFSET_BEGINNING;
    assert(afsplus_aros_packet_process(context, &packet) == 0);
    assert(packet.dp_Res1 == INT64_C(4294967301) && packet.dp_Res2 == 0);

    initialize_packet(&packet, ACTION_SEEK64);
    packet.dp_Arg1 = public_file.fh_Arg1;
    packet.dp_Arg2 = 0;
    packet.dp_Arg3 = OFFSET_BEGINNING;
    assert(afsplus_aros_packet_process(context, &packet) == 0);
    assert(packet.dp_Res1 == INT64_C(8589934592) && packet.dp_Res2 == 0);
    memset(read_data, 0, sizeof(read_data));
    initialize_packet(&packet, ACTION_READ);
    packet.dp_Arg1 = public_file.fh_Arg1;
    packet.dp_Arg2 = (SIPTR)read_data;
    packet.dp_Arg3 = sizeof(read_data);
    assert(afsplus_aros_packet_process(context, &packet) == 0);
    assert(packet.dp_Res1 == (SIPTR)sizeof(read_data) && packet.dp_Res2 == 0);
    assert(memcmp(read_data, "hello", sizeof(read_data)) == 0);

    memset(&fib, 0, sizeof(fib));
    initialize_packet(&packet, ACTION_EXAMINE_FH);
    packet.dp_Arg1 = public_file.fh_Arg1;
    packet.dp_Arg2 = (SIPTR)MKBADDR(&fib);
    assert(afsplus_aros_packet_process(context, &packet) == 0);
    assert(packet.dp_Res1 == DOSTRUE && packet.dp_Res2 == 0);
    assert(fib.fib_FileName[0] == 5);
    assert(memcmp(fib.fib_FileName + 1, "entry", 5) == 0);
    assert(fib.fib_Size == INT32_MAX);
    assert(fib.fib_Date.ds_Days == 1);
    assert(fib.fib_Date.ds_Minute == 1);
    assert(fib.fib_Date.ds_Tick == 52);

    memset(&fib64, 0, sizeof(fib64));
    initialize_packet(&packet, ACTION_EXAMINE_FH64);
    packet.dp_Arg1 = public_file.fh_Arg1;
    packet.dp_Arg2 = (SIPTR)MKBADDR(&fib64);
    assert(afsplus_aros_packet_process(context, &packet) == 0);
    assert(packet.dp_Res1 == DOSTRUE && packet.dp_Res2 == 0);
    assert(fib64.fib_Size == UINT64_C(0x100000002));

    memset(&disk, 0, sizeof(disk));
    initialize_packet(&packet, ACTION_DISK_INFO);
    packet.dp_Arg1 = (SIPTR)MKBADDR(&disk);
    assert(afsplus_aros_packet_process(context, &packet) == 0);
    assert(packet.dp_Res1 == DOSTRUE && packet.dp_Res2 == 0);
    assert(disk.id_NumBlocks == INT32_MAX);
    assert(disk.id_BytesPerBlock == 4096);

    memset(&disk64, 0, sizeof(disk64));
    initialize_packet(&packet, ACTION_INFO64);
    packet.dp_Arg2 = (SIPTR)MKBADDR(&disk64);
    assert(afsplus_aros_packet_process(context, &packet) == 0);
    assert(packet.dp_Res1 == DOSTRUE && packet.dp_Res2 == 0);
    assert(disk64.id_NumBlocks == UINT64_C(0x100000005));

    initialize_packet(&packet, ACTION_END);
    packet.dp_Arg1 = public_file.fh_Arg1;
    assert(afsplus_aros_packet_process(context, &packet) == 0);
    assert(packet.dp_Res1 == DOSTRUE && packet.dp_Res2 == 0);
    assert(fsync_count == 1 && close_count == 1);

    /* C2: metadata setters resolve a path to parent lock plus leaf. */
    {
        struct DateStamp stamp;

        reset_events();
        initialize_packet(&packet, ACTION_SET_PROTECT);
        packet.dp_Arg2 = (SIPTR)root;
        packet.dp_Arg3 = packet_bstr("dir/note");
        packet.dp_Arg4 = 0x71;
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == DOSTRUE && packet.dp_Res2 == 0);
        assert(stub_protection == 0x71);
        assert_event(0, 'L', "dir", AFSPLUS_AROS_LOCK_SHARED);
        assert_event(1, 'p', "note", 0);

        /* A path without a leaf addresses the resolved lock itself. */
        reset_events();
        initialize_packet(&packet, ACTION_SET_PROTECT);
        packet.dp_Arg2 = (SIPTR)root;
        packet.dp_Arg3 = packet_bstr("dir/");
        packet.dp_Arg4 = 0x0F;
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == DOSTRUE && packet.dp_Res2 == 0);
        assert(stub_protection == 0x0F);
        assert_event(0, 'L', "dir", AFSPLUS_AROS_LOCK_SHARED);
        assert_event(1, 'p', "", 0);

        /* 1978-01-02 00:01:01.04: day 1, minute 1, tick 52. */
        stamp.ds_Days = 1;
        stamp.ds_Minute = 1;
        stamp.ds_Tick = 52;
        reset_events();
        initialize_packet(&packet, ACTION_SET_DATE);
        packet.dp_Arg2 = (SIPTR)root;
        packet.dp_Arg3 = packet_bstr("note");
        packet.dp_Arg4 = (SIPTR)&stamp;
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == DOSTRUE && packet.dp_Res2 == 0);
        assert(stub_modified_seconds == INT64_C(252547261));
        assert(stub_modified_nanoseconds == UINT32_C(40000000));
        assert_event(0, 'd', "note", 0);

        stamp.ds_Tick = 3000;
        reset_events();
        initialize_packet(&packet, ACTION_SET_DATE);
        packet.dp_Arg2 = (SIPTR)root;
        packet.dp_Arg3 = packet_bstr("note");
        packet.dp_Arg4 = (SIPTR)&stamp;
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == DOSFALSE
            && packet.dp_Res2 == ERROR_BAD_NUMBER);
        assert(event_count == 0);
    }

    /* C2: soft links. */
    {
        char resolved[32];

        reset_events();
        initialize_packet(&packet, ACTION_MAKE_LINK);
        packet.dp_Arg1 = (SIPTR)root;
        packet.dp_Arg2 = packet_bstr("dir/alias");
        packet.dp_Arg3 = (SIPTR)"Work:real";
        packet.dp_Arg4 = LINK_SOFT;
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == DOSTRUE && packet.dp_Res2 == 0);
        assert_event(0, 'L', "dir", AFSPLUS_AROS_LOCK_SHARED);
        assert_event(1, 'S', "alias", 0);
        assert_event(2, 's', "Work:real", 0);

        /* A relative target replaces the link component in place. */
        stub_link_target = "real";
        memset(resolved, 0x7e, sizeof(resolved));
        initialize_packet(&packet, ACTION_READ_LINK);
        packet.dp_Arg1 = (SIPTR)root;
        packet.dp_Arg2 = (SIPTR)"dir/link/tail";
        packet.dp_Arg3 = (SIPTR)resolved;
        packet.dp_Arg4 = sizeof(resolved);
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == 13 && packet.dp_Res2 == 0);
        assert(memcmp(resolved, "dir/real/tail\0\x7e", 15) == 0);

        /* A target naming a volume discards the prefix. */
        stub_link_target = "Work:";
        initialize_packet(&packet, ACTION_READ_LINK);
        packet.dp_Arg1 = (SIPTR)root;
        packet.dp_Arg2 = (SIPTR)"dir/link/tail";
        packet.dp_Arg3 = (SIPTR)resolved;
        packet.dp_Arg4 = sizeof(resolved);
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == 9 && packet.dp_Res2 == 0);
        assert(strcmp(resolved, "Work:tail") == 0);

        /* One byte short of "dir/real/tail" plus NUL: -2, never truncation. */
        stub_link_target = "real";
        initialize_packet(&packet, ACTION_READ_LINK);
        packet.dp_Arg1 = (SIPTR)root;
        packet.dp_Arg2 = (SIPTR)"dir/link/tail";
        packet.dp_Arg3 = (SIPTR)resolved;
        packet.dp_Arg4 = 13;
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == -2
            && packet.dp_Res2 == ERROR_LINE_TOO_LONG);

        /* A path without any soft link is not a link. */
        initialize_packet(&packet, ACTION_READ_LINK);
        packet.dp_Arg1 = (SIPTR)root;
        packet.dp_Arg2 = (SIPTR)"dir/file";
        packet.dp_Arg3 = (SIPTR)resolved;
        packet.dp_Arg4 = sizeof(resolved);
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == -1
            && packet.dp_Res2 == ERROR_OBJECT_WRONG_TYPE);
    }

    /* C4: the extension packet. */
    {
        struct AfsplusExtRequest request;
        struct FileHandle writable_file;
        struct AfsplusArosCapabilities capabilities;
        struct AfsplusArosExtent extents[3];
        uint8_t data[8];
        uint64_t file_object;
        uint64_t root_id;

        memset(&writable_file, 0, sizeof(writable_file));
        initialize_packet(&packet, ACTION_FINDOUTPUT);
        packet.dp_Arg1 = (SIPTR)MKBADDR(&writable_file);
        packet.dp_Arg2 = (SIPTR)root;
        packet.dp_Arg3 = packet_bstr("ext");
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == DOSTRUE);
        file_object = (uint64_t)writable_file.fh_Arg1;

#define EXT_SEND(expected_res1, expected_res2) \
    do { \
        initialize_packet(&packet, ACTION_AFSPLUS_EXT); \
        packet.dp_Arg1 = (SIPTR)&request; \
        reset_events(); \
        assert(afsplus_aros_packet_process(context, &packet) == 0); \
        assert(packet.dp_Res1 == (expected_res1) \
            && packet.dp_Res2 == (expected_res2)); \
    } while (0)
#define EXT_BEGIN(op) \
    do { \
        memset(&request, 0, sizeof(request)); \
        request.magic = AFSPLUS_EXT_MAGIC; \
        request.version = AFSPLUS_EXT_VERSION; \
        request.header_size = sizeof(request); \
        request.operation = (op); \
    } while (0)

        /* The envelope is checked before anything is touched. */
        initialize_packet(&packet, ACTION_AFSPLUS_EXT);
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res2 == ERROR_REQUIRED_ARG_MISSING);
        EXT_BEGIN(AFSPLUS_EXT_INTERFACE);
        request.magic ^= 1;
        request.output_value = 0x7e7e;
        EXT_SEND(DOSFALSE, ERROR_BAD_NUMBER);
        assert(request.output_value == 0x7e7e);
        EXT_BEGIN(AFSPLUS_EXT_INTERFACE);
        request.version = 2;
        EXT_SEND(DOSFALSE, ERROR_BAD_NUMBER);
        EXT_BEGIN(AFSPLUS_EXT_INTERFACE);
        request.header_size = sizeof(request) - 1;
        EXT_SEND(DOSFALSE, ERROR_BAD_NUMBER);
        EXT_BEGIN(99);
        EXT_SEND(DOSFALSE, ERROR_BAD_NUMBER);
        assert(event_count == 0);

        /* A sender that owns only the eight-byte prefix, with a size that
         * says so: nothing behind the prefix is read. The bytes end where an
         * inaccessible page begins. */
        {
            struct AfsplusExtRequest prefix;
            uint8_t *short_block = before_guard_page(AFSPLUS_EXT_PREFIX_BYTES);

            memset(&prefix, 0, sizeof(prefix));
            prefix.magic = AFSPLUS_EXT_MAGIC;
            prefix.version = AFSPLUS_EXT_VERSION;
            prefix.header_size = AFSPLUS_EXT_PREFIX_BYTES;
            memcpy(short_block, &prefix, AFSPLUS_EXT_PREFIX_BYTES);
            initialize_packet(&packet, ACTION_AFSPLUS_EXT);
            packet.dp_Arg1 = (SIPTR)short_block;
            assert(afsplus_aros_packet_process(context, &packet) == 0);
            assert(packet.dp_Res1 == DOSFALSE
                && packet.dp_Res2 == ERROR_BAD_NUMBER);
        }

        /* Bits without a meaning are refused, so they can get one later. */
        EXT_BEGIN(AFSPLUS_EXT_INTERFACE);
        request.reserved = 1;
        EXT_SEND(DOSFALSE, ERROR_BAD_NUMBER);
        EXT_BEGIN(AFSPLUS_EXT_INTERFACE);
        request.flags = 1;
        EXT_SEND(DOSFALSE, ERROR_BAD_NUMBER);

        /* A longer block from a newer client is served by its known part. */
        EXT_BEGIN(AFSPLUS_EXT_INTERFACE);
        request.header_size = sizeof(request) + 16;
        EXT_SEND(DOSTRUE, 0);
        assert(request.output_count == stub_revision);
        assert(request.output_flags == AFSPLUS_AROS_PACKET_ABI_VERSION);
        assert(request.output_value == stub_groups);

        /* A struct buffer must hold the size it declares. */
        memset(&capabilities, 0, sizeof(capabilities));
        capabilities.struct_size = sizeof(capabilities);
        EXT_BEGIN(AFSPLUS_EXT_CAPABILITIES);
        request.buffer = &capabilities;
        request.buffer_size = sizeof(capabilities) - 1;
        EXT_SEND(DOSFALSE, ERROR_BAD_NUMBER);
        assert(event_count == 0);
        request.buffer_size = sizeof(capabilities);
        EXT_SEND(DOSTRUE, 0);
        assert(capabilities.mount_mode == 77);
        assert(ext_seen[0] == sizeof(capabilities));

        /* The declared size is read once. A writer that enlarges it during
         * the call moves nothing: the entry point never sees this buffer,
         * and exactly the size read at entry is copied back. */
        {
            union { uint8_t bytes[96]; struct AfsplusArosCapabilities c; }
                guarded;
            /* A newer client: its struct has sixteen bytes this handler
             * does not know. */
            uint32_t declared = sizeof(struct AfsplusArosCapabilities) + 16;
            size_t at;

            memset(&guarded, 0x7e, sizeof(guarded));
            memcpy(guarded.bytes, &declared, sizeof(declared));
            ext_caller_buffer = guarded.bytes;
            EXT_BEGIN(AFSPLUS_EXT_CAPABILITIES);
            request.buffer = guarded.bytes;
            request.buffer_size = sizeof(guarded);
            EXT_SEND(DOSTRUE, 0);
            ext_caller_buffer = NULL;
            assert(guarded.c.struct_size == sizeof(guarded.c)
                && guarded.c.mount_mode == 77);
            for (at = sizeof(guarded.c); at < sizeof(guarded); at++)
                assert(guarded.bytes[at] == 0x7e);

            /* Below the first published layout the entry point refuses, and
             * the transport hands that answer on with the buffer untouched. */
            declared = AFSPLUS_AROS_CAPABILITIES_FIRST_LAYOUT - 1;
            memset(&guarded, 0x7e, sizeof(guarded));
            memcpy(guarded.bytes, &declared, sizeof(declared));
            EXT_SEND(DOSFALSE, ERROR_BAD_NUMBER);
            for (at = sizeof(declared); at < sizeof(guarded); at++)
                assert(guarded.bytes[at] == 0x7e);
        }

        /* Positioned I/O on the application's fh_Arg1. */
        EXT_BEGIN(AFSPLUS_EXT_READ_AT);
        request.object[0] = file_object;
        request.offset[0] = UINT64_C(0x500000000);
        request.buffer = data;
        request.buffer_size = sizeof(data);
        EXT_SEND(DOSTRUE, 0);
        assert(events[0].operation == '2' && events[0].base >= 100);
        assert(ext_seen[0] == UINT64_C(0x500000000) && ext_seen[1] == 8);
        assert(request.output_count == 2 && data[0] == 'o');
        request.operation = AFSPLUS_EXT_WRITE_AT;
        EXT_SEND(DOSTRUE, 0);
        assert(events[0].operation == '3' && request.output_count == 8);
        /* Not a file of this handler, and a lock is not a file. */
        request.object[0] = (uint64_t)root;
        EXT_SEND(DOSFALSE, ERROR_INVALID_LOCK);
        request.object[0] = file_object;
        request.buffer = NULL;
        EXT_SEND(DOSFALSE, ERROR_BAD_NUMBER);
        assert(event_count == 0);

        /* Locks travel as BPTRs; zero is the root for a base, never for the
         * clone source. */
        initialize_packet(&packet, ACTION_LOCATE_OBJECT);
        packet.dp_Arg1 = (SIPTR)root;
        packet.dp_Arg2 = packet_bstr("");
        packet.dp_Arg3 = SHARED_LOCK;
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        EXT_BEGIN(AFSPLUS_EXT_CLONE_FILE);
        request.object[0] = (uint64_t)packet.dp_Res1;
        request.name1 = (const uint8_t *)"copy";
        request.name_length[1] = 4;
        {
            BPTR source = (BPTR)packet.dp_Res1;

            EXT_SEND(DOSTRUE, 0);
            assert_event(0, '4', "copy", 0);
            root_id = events[0].base;
            assert(root_id != 0 && ext_seen[0] == 0);
            request.object[0] = 0;
            EXT_SEND(DOSFALSE, ERROR_INVALID_LOCK);
            request.object[0] = file_object;
            EXT_SEND(DOSFALSE, ERROR_INVALID_LOCK);
            assert(event_count == 0);
            initialize_packet(&packet, ACTION_FREE_LOCK);
            packet.dp_Arg1 = (SIPTR)source;
            assert(afsplus_aros_packet_process(context, &packet) == 0);
        }

        /* A name length is bounded before its bytes are read: the fakes are
         * never reached with a 4 GiB claim, nor with one byte too many. */
        EXT_BEGIN(AFSPLUS_EXT_LOOKUP_ID);
        request.name0 = (const uint8_t *)"x";
        request.name_length[0] = UINT32_MAX;
        EXT_SEND(DOSFALSE, ERROR_INVALID_COMPONENT_NAME);
        assert(event_count == 0);
        request.name_length[0] = AFSPLUS_EXT_NAME_MAX + 1;
        EXT_SEND(DOSFALSE, ERROR_INVALID_COMPONENT_NAME);
        EXT_BEGIN(AFSPLUS_EXT_REPLACE);
        request.name0 = (const uint8_t *)"tmp";
        request.name_length[0] = 3;
        request.name1 = (const uint8_t *)"x";
        request.name_length[1] = UINT32_MAX;
        EXT_SEND(DOSFALSE, ERROR_INVALID_COMPONENT_NAME);
        EXT_BEGIN(AFSPLUS_EXT_CLONE_FILE);
        request.object[0] = (uint64_t)root;
        request.name1 = (const uint8_t *)"x";
        request.name_length[1] = UINT32_MAX;
        EXT_SEND(DOSFALSE, ERROR_INVALID_COMPONENT_NAME);
        assert(event_count == 0);

        EXT_BEGIN(AFSPLUS_EXT_CLONE_RANGE);
        request.object[0] = file_object;
        request.object[1] = file_object;
        request.offset[0] = 4096;
        request.offset[1] = 8192;
        request.length = UINT64_C(0x100000000);
        EXT_SEND(DOSTRUE, 0);
        assert(events[0].operation == '5' && ext_seen[0] == 4096
            && ext_seen[2] == 8192 && ext_seen[3] == UINT64_C(0x100000000));

        EXT_BEGIN(AFSPLUS_EXT_PREALLOCATE);
        request.object[0] = file_object;
        request.offset[0] = 1;
        request.length = 2;
        EXT_SEND(DOSTRUE, 0);
        assert(events[0].operation == '6' && ext_seen[1] == 2);

        EXT_BEGIN(AFSPLUS_EXT_REPLACE);
        request.name0 = (const uint8_t *)"tmp";
        request.name_length[0] = 3;
        request.name1 = (const uint8_t *)"new";
        request.name_length[1] = 3;
        EXT_SEND(DOSTRUE, 0);
        assert_event(0, '7', "tmp", 0);
        assert(ext_seen[4] == 1);
        request.name1 = NULL;
        EXT_SEND(DOSFALSE, ERROR_BAD_NUMBER);

        EXT_BEGIN(AFSPLUS_EXT_ADVISE);
        request.object[0] = file_object;
        request.flags = 3;
        EXT_SEND(DOSTRUE, 0);
        assert(ext_seen[2] == 3 && request.output_flags == 9);

        /* A short JSON buffer learns the size it needs. */
        EXT_BEGIN(AFSPLUS_EXT_INFO_JSON);
        EXT_SEND(DOSTRUE, 0);
        assert(request.output_value == 4 && ext_seen[0] == 0);
        request.buffer = data;
        request.buffer_size = sizeof(data);
        EXT_SEND(DOSTRUE, 0);
        assert(data[0] == '{');

        /* The extent array is sized in bytes; a partial element is unused. */
        EXT_BEGIN(AFSPLUS_EXT_EXTENT_MAP);
        request.object[0] = file_object;
        request.offset[0] = 10;
        request.length = 20;
        request.buffer = extents;
        request.buffer_size = sizeof(extents) - 1;
        EXT_SEND(DOSTRUE, 0);
        assert(ext_seen[2] == 2 && request.output_count == 2);
        assert(request.output_flags == 1 && request.output_value == 30);

        /* A buffer for a hundred extents is served with sixty-four. */
        {
            static struct AfsplusArosExtent many_extents[100];

            request.buffer = many_extents;
            request.buffer_size = sizeof(many_extents);
            EXT_SEND(DOSTRUE, 0);
            assert(ext_seen[2] == 64 && request.output_count == 64);
        }

        EXT_BEGIN(AFSPLUS_EXT_LOOKUP_ID);
        request.name0 = (const uint8_t *)"ext";
        request.name_length[0] = 3;
        EXT_SEND(DOSTRUE, 0);
        assert_event(0, 'd', "ext", 0);
        assert(request.output_value == UINT64_C(0x1122334455));

        /* A filesystem error travels in dp_Res2 and clears the outputs. */
        ext_error = ERROR_OBJECT_NOT_FOUND;
        request.output_value = 5;
        EXT_SEND(DOSFALSE, ERROR_OBJECT_NOT_FOUND);
        /* A volume without the capability says "unknown action" at the C
         * boundary. Through the transport that value would tell the client
         * that the packet is unknown, and it would fall back. */
        ext_error = ERROR_ACTION_NOT_KNOWN;
        EXT_BEGIN(AFSPLUS_EXT_CLONE_RANGE);
        request.object[0] = file_object;
        request.object[1] = file_object;
        EXT_SEND(DOSFALSE, ERROR_NOT_IMPLEMENTED);
        ext_error = 0;

        initialize_packet(&packet, ACTION_END);
        packet.dp_Arg1 = writable_file.fh_Arg1;
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        (void)root_id;
    }

    /* C2: ACTION_SET_COMMENT, and the comment in Examine, ExNext, ExamineFH
     * and ExAll records. */
    {
        union { struct FileInfoBlock fib; void *align; } block;
        union { uint8_t bytes[512]; void *align; } buffer;
        struct FileHandle commented;
        struct ExAllControl control;
        struct ExAllData *entry;
        char text[81];
        size_t first;

        reset_events();
        initialize_packet(&packet, ACTION_SET_COMMENT);
        packet.dp_Arg2 = (SIPTR)root;
        packet.dp_Arg3 = packet_bstr("dir/note");
        packet.dp_Arg4 = packet_bstr("draft two");
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == DOSTRUE && packet.dp_Res2 == 0);
        assert_event(0, 'L', "dir", AFSPLUS_AROS_LOCK_SHARED);
        assert_event(1, 'C', "note", 0);
        assert(stub_comment_length == 9
            && memcmp(stub_comment, "draft two", 9) == 0);

        /* 79 characters is the most fib_Comment holds; 80 is refused before
         * the filesystem is reached and the stored comment stays. */
        memset(text, 'k', 80);
        text[80] = 0;
        reset_events();
        initialize_packet(&packet, ACTION_SET_COMMENT);
        packet.dp_Arg2 = (SIPTR)root;
        packet.dp_Arg3 = packet_bstr("note");
        packet.dp_Arg4 = packet_bstr(text);
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == DOSFALSE
            && packet.dp_Res2 == ERROR_COMMENT_TOO_BIG);
        assert(event_count == 0 && stub_comment_length == 9);
        text[79] = 0;
        packet.dp_Arg4 = packet_bstr(text);
        initialize_packet(&packet, ACTION_SET_COMMENT);
        packet.dp_Arg2 = (SIPTR)root;
        packet.dp_Arg3 = packet_bstr("note");
        packet.dp_Arg4 = packet_bstr(text);
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == DOSTRUE && stub_comment_length == 79);

        /* Examine: the lock's own object, addressed by the empty name. The
         * 79-character comment fills fib_Comment to its last byte. */
        reset_events();
        memset(&block, 0x7e, sizeof(block));
        initialize_packet(&packet, ACTION_EXAMINE_OBJECT);
        packet.dp_Arg1 = (SIPTR)root;
        packet.dp_Arg2 = (SIPTR)MKBADDR(&block.fib);
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == DOSTRUE && packet.dp_Res2 == 0);
        assert_event(0, 'c', "", 0);
        assert(block.fib.fib_Comment[0] == 79);
        assert(memcmp(block.fib.fib_Comment + 1, text, 79) == 0);
        assert(sizeof(block.fib.fib_Comment) == 80);

        /* ExNext: the entry is named under the directory lock. */
        memcpy(stub_comment, "next", 4);
        stub_comment_length = 4;
        reset_events();
        initialize_packet(&packet, ACTION_EXAMINE_NEXT);
        packet.dp_Arg1 = (SIPTR)root;
        packet.dp_Arg2 = (SIPTR)MKBADDR(&block.fib);
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == DOSTRUE);
        assert_event(0, 'c', "entry", 0);
        assert(block.fib.fib_Comment[0] == 4
            && memcmp(block.fib.fib_Comment + 1, "next", 5) == 0);

        /* ExamineFH has no lock and asks by file. */
        memset(&commented, 0, sizeof(commented));
        initialize_packet(&packet, ACTION_FINDOUTPUT);
        packet.dp_Arg1 = (SIPTR)MKBADDR(&commented);
        packet.dp_Arg2 = (SIPTR)root;
        packet.dp_Arg3 = packet_bstr("note");
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == DOSTRUE);
        reset_events();
        initialize_packet(&packet, ACTION_EXAMINE_FH);
        packet.dp_Arg1 = commented.fh_Arg1;
        packet.dp_Arg2 = (SIPTR)MKBADDR(&block.fib);
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == DOSTRUE);
        assert(events[0].operation == 'c' && events[0].access == 1);
        assert(block.fib.fib_Comment[0] == 4);
        initialize_packet(&packet, ACTION_END);
        packet.dp_Arg1 = commented.fh_Arg1;
        assert(afsplus_aros_packet_process(context, &packet) == 0);

        /* A comment that cannot be read fails the Examine: the block is not
         * reported as commentless. */
        stub_comment_error = ERROR_SEEK_ERROR;
        initialize_packet(&packet, ACTION_EXAMINE_OBJECT);
        packet.dp_Arg1 = (SIPTR)root;
        packet.dp_Arg2 = (SIPTR)MKBADDR(&block.fib);
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == DOSFALSE
            && packet.dp_Res2 == ERROR_SEEK_ERROR);
        stub_comment_error = 0;

        /* ExAll: ED_COMMENT records carry the comment after the name and its
         * bytes count toward the fit; ED_DATE records never ask for it. */
        stub_directory_size = 2;
        memset(&control, 0, sizeof(control));
        memset(&buffer, 0x7e, sizeof(buffer));
        first = offsetof(struct ExAllData, ed_OwnerUID) + 3 + 5;
        reset_events();
        initialize_packet(&packet, ACTION_EXAMINE_ALL);
        packet.dp_Arg1 = (SIPTR)root;
        packet.dp_Arg2 = (SIPTR)buffer.bytes;
        /* One byte short of the first record with its comment. */
        packet.dp_Arg3 = (SIPTR)(first - 1);
        packet.dp_Arg4 = ED_COMMENT;
        packet.dp_Arg5 = (SIPTR)&control;
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == DOSFALSE
            && packet.dp_Res2 == ERROR_BUFFER_OVERFLOW);
        assert(control.eac_Entries == 0);
        packet.dp_Arg3 = (SIPTR)sizeof(buffer);
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == DOSFALSE
            && packet.dp_Res2 == ERROR_NO_MORE_ENTRIES);
        assert(control.eac_Entries == 2);
        entry = (struct ExAllData *)buffer.bytes;
        assert(strcmp((char *)entry->ed_Name, "e0") == 0);
        assert(entry->ed_Comment == entry->ed_Name + 3);
        assert(strcmp((char *)entry->ed_Comment, "next") == 0);
        assert(strcmp((char *)entry->ed_Next->ed_Comment, "next") == 0);
        assert_event(0, 'c', "e0", 0);

        memset(&control, 0, sizeof(control));
        reset_events();
        packet.dp_Arg4 = ED_DATE;
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(control.eac_Entries == 2 && event_count == 0);

        stub_comment_length = 0;
        stub_directory_size = -1;
    }

    /* C2: ACTION_EXAMINE_ALL over a five-entry directory. */
    {
        /* ED_DATE entry with a two-character name: fixed part up to
         * ed_Comment plus "eN\0", pointer-aligned. */
        const size_t one = (offsetof(struct ExAllData, ed_Comment) + 3
            + sizeof(void *) - 1) & ~(sizeof(void *) - 1);
        union { uint8_t bytes[256]; void *align; } buffer;
        struct ExAllControl control;
        struct ExAllData *entry;
        uint32_t rewinds;

        stub_directory_size = 5;
        memset(&control, 0, sizeof(control));
        memset(&buffer, 0x7e, sizeof(buffer));
        rewinds = rewind_count;

        /* Room for exactly two entries: the third is read, kept pending and
         * must come back first on the next call, never be lost. */
        initialize_packet(&packet, ACTION_EXAMINE_ALL);
        packet.dp_Arg1 = (SIPTR)root;
        packet.dp_Arg2 = (SIPTR)buffer.bytes;
        /* One byte short of the third entry's own bytes. */
        packet.dp_Arg3 = (SIPTR)(2 * one
            + offsetof(struct ExAllData, ed_Comment) + 3 - 1);
        packet.dp_Arg4 = ED_DATE;
        packet.dp_Arg5 = (SIPTR)&control;
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == DOSTRUE && packet.dp_Res2 == 0);
        assert(control.eac_Entries == 2 && control.eac_LastKey != 0);
        assert(rewind_count == rewinds + 1);
        entry = (struct ExAllData *)buffer.bytes;
        assert(strcmp((char *)entry->ed_Name, "e0") == 0);
        assert(entry->ed_Type == ST_LINKFILE && entry->ed_Size == 10);
        assert(entry->ed_Prot == 0x40);
        assert(entry->ed_Days == 1 && entry->ed_Mins == 1
            && entry->ed_Ticks == 52);
        assert((uint8_t *)entry->ed_Next == buffer.bytes + one);
        entry = entry->ed_Next;
        assert(strcmp((char *)entry->ed_Name, "e1") == 0);
        assert(entry->ed_Size == 11 && entry->ed_Next == NULL);
        /* Nothing was written past the second entry. */
        assert(buffer.bytes[2 * one] == 0x7e);

        /* Continuation: no rewind, e2 first, then e3 and e4, then the end. */
        packet.dp_Arg3 = (SIPTR)sizeof(buffer);
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == DOSFALSE
            && packet.dp_Res2 == ERROR_NO_MORE_ENTRIES);
        assert(control.eac_Entries == 3);
        assert(rewind_count == rewinds + 1);
        entry = (struct ExAllData *)buffer.bytes;
        assert(strcmp((char *)entry->ed_Name, "e2") == 0);
        assert(strcmp((char *)entry->ed_Next->ed_Name, "e3") == 0);
        assert(strcmp((char *)entry->ed_Next->ed_Next->ed_Name, "e4") == 0);
        assert(entry->ed_Next->ed_Next->ed_Next == NULL);

        /* ED_NAME entries carry the name only; a zero key restarts. */
        control.eac_LastKey = 0;
        packet.dp_Arg4 = ED_NAME;
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res2 == ERROR_NO_MORE_ENTRIES);
        assert(control.eac_Entries == 5 && rewind_count == rewinds + 2);
        entry = (struct ExAllData *)buffer.bytes;
        assert(strcmp((char *)entry->ed_Name, "e0") == 0);
        assert((uint8_t *)entry->ed_Name
            == buffer.bytes + offsetof(struct ExAllData, ed_Type));

        /* A buffer too small for one entry keeps the entry and says so. */
        control.eac_LastKey = 0;
        packet.dp_Arg3 = 4;
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == DOSFALSE
            && packet.dp_Res2 == ERROR_BUFFER_OVERFLOW);
        assert(control.eac_Entries == 0);
        packet.dp_Arg3 = (SIPTR)sizeof(buffer);
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(control.eac_Entries == 5);
        assert(strcmp((char *)((struct ExAllData *)buffer.bytes)->ed_Name,
            "e0") == 0);

        /* Patterns need dos.library: refused so that dos.library emulates.
         * Unknown detail levels are ERROR_BAD_NUMBER. */
        control.eac_MatchString = (UBYTE *)"#?";
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == DOSFALSE
            && packet.dp_Res2 == ERROR_ACTION_NOT_KNOWN);
        control.eac_MatchString = NULL;
        packet.dp_Arg4 = ED_OWNER + 1;
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res2 == ERROR_BAD_NUMBER);

        /* An entry whose own bytes exactly fill the buffer fits, although
         * its alignment padding would not: two pointers plus "e0" and NUL. */
        memset(&buffer, 0x7e, sizeof(buffer));
        control.eac_LastKey = 0;
        control.eac_Entries = 7;
        packet.dp_Arg3 = (SIPTR)(offsetof(struct ExAllData, ed_Type) + 3);
        packet.dp_Arg4 = ED_NAME;
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == DOSTRUE && control.eac_Entries == 1);
        assert(strcmp((char *)((struct ExAllData *)buffer.bytes)->ed_Name,
            "e0") == 0);
        assert(buffer.bytes[offsetof(struct ExAllData, ed_Type) + 3] == 0x7e);

        /* An error return reports zero entries, never the previous count. */
        control.eac_Entries = 7;
        packet.dp_Arg4 = ED_OWNER + 1;
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res2 == ERROR_BAD_NUMBER && control.eac_Entries == 0);

        /* A read failure after two packed entries returns those two; the
         * failure surfaces on the next call with an empty result. */
        control.eac_LastKey = 0;
        stub_directory_fail_at = 2;
        packet.dp_Arg3 = (SIPTR)sizeof(buffer);
        packet.dp_Arg4 = ED_NAME;
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == DOSTRUE && packet.dp_Res2 == 0);
        assert(control.eac_Entries == 2);
        stub_directory_fail_at = 2;
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == DOSFALSE
            && packet.dp_Res2 == ERROR_SEEK_ERROR);
        assert(control.eac_Entries == 0);

        /* One lock has one directory cursor. A second sequence takes it
         * over, and the first one's continuation is refused by value rather
         * than served from the second one's position. */
        {
            struct ExAllControl other;
            struct FileInfoBlock interleaved;

            memset(&other, 0, sizeof(other));
            stub_directory_size = 8;
            control.eac_LastKey = 0;
            packet.dp_Arg3 = (SIPTR)(2 * one);
            packet.dp_Arg4 = ED_DATE;
            assert(afsplus_aros_packet_process(context, &packet) == 0);
            assert(packet.dp_Res1 == DOSTRUE && control.eac_Entries == 2);

            packet.dp_Arg5 = (SIPTR)&other;
            assert(afsplus_aros_packet_process(context, &packet) == 0);
            assert(packet.dp_Res1 == DOSTRUE && other.eac_Entries == 2);
            assert(other.eac_LastKey != control.eac_LastKey);
            assert(strcmp((char *)((struct ExAllData *)buffer.bytes)->ed_Name,
                "e0") == 0);

            packet.dp_Arg5 = (SIPTR)&control;
            assert(afsplus_aros_packet_process(context, &packet) == 0);
            assert(packet.dp_Res1 == DOSFALSE
                && packet.dp_Res2 == ERROR_OBJECT_IN_USE);

            /* The owner continues exactly where it was: e2, e3. */
            packet.dp_Arg5 = (SIPTR)&other;
            assert(afsplus_aros_packet_process(context, &packet) == 0);
            assert(packet.dp_Res1 == DOSTRUE && other.eac_Entries == 2);
            assert(strcmp((char *)((struct ExAllData *)buffer.bytes)->ed_Name,
                "e2") == 0);

            /* ExNext on the same lock moves the cursor, so it ends the
             * sequence: the continuation is refused, not silently short. */
            {
                struct DosPacket next;

                initialize_packet(&next, ACTION_EXAMINE_NEXT);
                next.dp_Arg1 = (SIPTR)root;
                next.dp_Arg2 = (SIPTR)MKBADDR(&interleaved);
                assert(afsplus_aros_packet_process(context, &next) == 0);
                assert(next.dp_Res1 == DOSTRUE);
            }
            assert(afsplus_aros_packet_process(context, &packet) == 0);
            assert(packet.dp_Res1 == DOSFALSE
                && packet.dp_Res2 == ERROR_OBJECT_IN_USE);
            packet.dp_Arg5 = (SIPTR)&control;
        }

        initialize_packet(&packet, ACTION_EXAMINE_ALL_END);
        packet.dp_Arg1 = (SIPTR)root;
        rewinds = rewind_count;
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == DOSTRUE && rewind_count == rewinds + 1);
        stub_directory_size = -1;
    }

    /* C2: OpenFromLock consumes the lock wrapper; ChangeMode maps DOS modes. */
    {
        struct FileHandle from_lock;
        BPTR file_lock;
        uint32_t closes = close_count;

        initialize_packet(&packet, ACTION_LOCATE_OBJECT);
        packet.dp_Arg1 = (SIPTR)root;
        packet.dp_Arg2 = packet_bstr("data");
        packet.dp_Arg3 = SHARED_LOCK;
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        file_lock = (BPTR)packet.dp_Res1;
        assert(file_lock != BNULL);

        /* A refused conversion leaves the lock usable. */
        memset(&from_lock, 0, sizeof(from_lock));
        stub_open_from_lock_error = ERROR_OBJECT_WRONG_TYPE;
        initialize_packet(&packet, ACTION_FH_FROM_LOCK);
        packet.dp_Arg1 = (SIPTR)MKBADDR(&from_lock);
        packet.dp_Arg2 = (SIPTR)file_lock;
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == DOSFALSE
            && packet.dp_Res2 == ERROR_OBJECT_WRONG_TYPE);
        assert(from_lock.fh_Arg1 == 0);
        stub_open_from_lock_error = 0;

        initialize_packet(&packet, ACTION_CHANGE_MODE);
        packet.dp_Arg1 = CHANGE_LOCK;
        packet.dp_Arg2 = (SIPTR)file_lock;
        packet.dp_Arg3 = EXCLUSIVE_LOCK;
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == DOSTRUE);
        assert(stub_changed_access == AFSPLUS_AROS_LOCK_EXCLUSIVE);
        assert(((struct FileLock *)BADDR(file_lock))->fl_Access
            == EXCLUSIVE_LOCK);
        /* A refused change keeps the published access. */
        stub_change_mode_error = ERROR_OBJECT_IN_USE;
        packet.dp_Arg3 = SHARED_LOCK;
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == DOSFALSE
            && packet.dp_Res2 == ERROR_OBJECT_IN_USE);
        assert(((struct FileLock *)BADDR(file_lock))->fl_Access
            == EXCLUSIVE_LOCK);
        stub_change_mode_error = 0;
        packet.dp_Arg3 = 12345;
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res2 == ERROR_BAD_NUMBER);

        /* Conversion: the handle is published, the lock BPTR is dead and the
         * filesystem lock is never freed separately. */
        reset_events();
        initialize_packet(&packet, ACTION_FH_FROM_LOCK);
        packet.dp_Arg1 = (SIPTR)MKBADDR(&from_lock);
        packet.dp_Arg2 = (SIPTR)file_lock;
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == DOSTRUE && from_lock.fh_Arg1 != 0);
        assert(events[event_count - 1].operation == 'O');
        initialize_packet(&packet, ACTION_FREE_LOCK);
        packet.dp_Arg1 = (SIPTR)file_lock;
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == DOSFALSE
            && packet.dp_Res2 == ERROR_INVALID_LOCK);

        /* MODE_NEWFILE on a handle means exclusive. */
        initialize_packet(&packet, ACTION_CHANGE_MODE);
        packet.dp_Arg1 = CHANGE_FH;
        packet.dp_Arg2 = (SIPTR)MKBADDR(&from_lock);
        packet.dp_Arg3 = MODE_NEWFILE;
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == DOSTRUE);
        assert(events[event_count - 1].operation == 'M');
        assert(stub_changed_access == AFSPLUS_AROS_LOCK_EXCLUSIVE);
        packet.dp_Arg1 = 2;
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res2 == ERROR_BAD_NUMBER);

        /* Record locks on that handle: unsigned 32-bit offsets, mode mapping,
         * and the collision reported by waiting and immediate modes. */
        reset_events();
        initialize_packet(&packet, ACTION_LOCK_RECORD);
        packet.dp_Arg1 = from_lock.fh_Arg1;
        packet.dp_Arg2 = (SIPTR)UINT32_C(0xF0000000);
        packet.dp_Arg3 = 64;
        packet.dp_Arg4 = REC_EXCLUSIVE_IMMED;
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == DOSTRUE && packet.dp_Res2 == 0);
        assert(stub_record_offset == UINT64_C(0xF0000000));
        assert(stub_record_length == 64 && stub_record_exclusive == 1);
        packet.dp_Arg4 = REC_SHARED;
        stub_record_error = ERROR_LOCK_COLLISION;
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(stub_record_exclusive == 0);
        assert(packet.dp_Res1 == DOSFALSE
            && packet.dp_Res2 == ERROR_LOCK_TIMEOUT);
        packet.dp_Arg4 = REC_SHARED_IMMED;
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res2 == ERROR_LOCK_COLLISION);
        stub_record_error = 0;
        packet.dp_Arg4 = REC_SHARED_IMMED + 1;
        event_count = 0;
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res2 == ERROR_BAD_NUMBER && event_count == 0);
        /* The 64-bit packets carry full-width ranges; the classic packet
         * given the same bits keeps only the low 32. */
        initialize_packet(&packet, ACTION_LOCK_RECORD64);
        packet.dp_Arg1 = from_lock.fh_Arg1;
        packet.dp_Arg2 = (SIPTR)INT64_C(0x500000010);
        packet.dp_Arg3 = (SIPTR)INT64_C(0x200000000);
        packet.dp_Arg4 = REC_SHARED_IMMED;
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == DOSTRUE);
        assert(events[event_count - 1].operation == 'k');
        assert(stub_record_offset == UINT64_C(0x500000010));
        assert(stub_record_length == UINT64_C(0x200000000));
        packet.dp_Type = ACTION_LOCK_RECORD;
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(stub_record_offset == UINT64_C(0x10));
        assert(stub_record_length == 0);
        initialize_packet(&packet, ACTION_FREE_RECORD64);
        packet.dp_Arg1 = from_lock.fh_Arg1;
        packet.dp_Arg2 = (SIPTR)INT64_C(0x500000010);
        packet.dp_Arg3 = (SIPTR)INT64_C(0x200000000);
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == DOSTRUE);
        assert(events[event_count - 1].operation == 'u');
        assert(stub_record_offset == UINT64_C(0x500000010));

        initialize_packet(&packet, ACTION_FREE_RECORD);
        packet.dp_Arg1 = from_lock.fh_Arg1;
        packet.dp_Arg2 = 8;
        packet.dp_Arg3 = 16;
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == DOSTRUE);
        assert(events[event_count - 1].operation == 'u');
        assert(stub_record_offset == 8 && stub_record_length == 16);
        packet.dp_Arg1 = 0;
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res2 == ERROR_INVALID_LOCK);

        /* Waiting modes with a timeout: the packet is kept, not answered,
         * and comes back through the complete callback. */
        {
            struct DosPacket first;
            struct DosPacket second;
            struct DosPacket third;
            struct FileHandle other;
            struct DosPacket many[17];
            size_t index;

            completed_count = 0;
            stub_record_error = ERROR_LOCK_COLLISION;
            initialize_packet(&first, ACTION_LOCK_RECORD);
            first.dp_Arg1 = from_lock.fh_Arg1;
            first.dp_Arg2 = 100;
            first.dp_Arg3 = 10;
            first.dp_Arg4 = REC_EXCLUSIVE;
            first.dp_Arg5 = 10;
            first.dp_Res1 = 0x5a5a;
            first.dp_Res2 = 0x5a5a;
            assert(afsplus_aros_packet_process(context, &first)
                == AFSPLUS_AROS_PACKET_DEFERRED);
            assert(first.dp_Res1 == 0x5a5a && first.dp_Res2 == 0x5a5a);
            second = first;
            second.dp_Arg2 = 200;
            second.dp_Arg4 = REC_SHARED;
            second.dp_Arg5 = 30;
            assert(afsplus_aros_packet_process(context, &second)
                == AFSPLUS_AROS_PACKET_DEFERRED);
            assert(afsplus_aros_packet_waiting(context) == 2);
            assert(completed_count == 0);

            /* Nine ticks expire nobody; the tenth expires the first only. */
            afsplus_aros_packet_elapsed(context, 9);
            assert(completed_count == 0);
            afsplus_aros_packet_elapsed(context, 1);
            assert(completed_count == 1 && completed[0] == &first);
            assert(first.dp_Res1 == DOSFALSE
                && first.dp_Res2 == ERROR_LOCK_TIMEOUT);
            assert(afsplus_aros_packet_waiting(context) == 1);

            /* A free that does not release the awaited range: the waiter is
             * retried, collides again and keeps its place and its time. */
            stub_free_succeeds = 1;
            reset_events();
            initialize_packet(&packet, ACTION_FREE_RECORD);
            packet.dp_Arg1 = from_lock.fh_Arg1;
            packet.dp_Arg2 = 8;
            packet.dp_Arg3 = 16;
            assert(afsplus_aros_packet_process(context, &packet) == 0);
            assert(packet.dp_Res1 == DOSTRUE);
            assert(event_count == 2 && events[0].operation == 'u'
                && events[1].operation == 'k' && events[1].access == 0);
            assert(completed_count == 1);
            assert(afsplus_aros_packet_waiting(context) == 1);

            /* The free that releases it: the waiter is granted before the
             * freeing packet returns, with the range it asked for. */
            stub_record_error = 0;
            assert(afsplus_aros_packet_process(context, &packet) == 0);
            assert(completed_count == 2 && completed[1] == &second);
            assert(second.dp_Res1 == DOSTRUE && second.dp_Res2 == 0);
            assert(stub_record_offset == 200 && stub_record_exclusive == 0);
            assert(afsplus_aros_packet_waiting(context) == 0);
            /* Twenty ticks were left; time passing later touches nothing. */
            afsplus_aros_packet_elapsed(context, 1000);
            assert(completed_count == 2);

            /* A retry that fails for another reason ends the wait with that
             * reason. */
            stub_record_error = ERROR_LOCK_COLLISION;
            third = first;
            assert(afsplus_aros_packet_process(context, &third)
                == AFSPLUS_AROS_PACKET_DEFERRED);
            stub_record_error = ERROR_NO_FREE_STORE;
            assert(afsplus_aros_packet_process(context, &packet) == 0);
            assert(completed_count == 3 && completed[2] == &third);
            assert(third.dp_Res1 == DOSFALSE
                && third.dp_Res2 == ERROR_NO_FREE_STORE);

            /* A file that closes takes its waiting packet with it, before the
             * filesystem forgets the handle; another file's waiter stays. */
            memset(&other, 0, sizeof(other));
            initialize_packet(&packet, ACTION_FINDOUTPUT);
            packet.dp_Arg1 = (SIPTR)MKBADDR(&other);
            packet.dp_Arg2 = (SIPTR)root;
            packet.dp_Arg3 = packet_bstr("other");
            assert(afsplus_aros_packet_process(context, &packet) == 0);
            assert(packet.dp_Res1 == DOSTRUE);
            stub_record_error = ERROR_LOCK_COLLISION;
            third = first;
            third.dp_Arg1 = other.fh_Arg1;
            assert(afsplus_aros_packet_process(context, &third)
                == AFSPLUS_AROS_PACKET_DEFERRED);
            second = first;
            assert(afsplus_aros_packet_process(context, &second)
                == AFSPLUS_AROS_PACKET_DEFERRED);
            initialize_packet(&packet, ACTION_END);
            packet.dp_Arg1 = other.fh_Arg1;
            assert(afsplus_aros_packet_process(context, &packet) == 0);
            assert(packet.dp_Res1 == DOSTRUE);
            assert(completed_count == 4 && completed[3] == &third);
            assert(third.dp_Res2 == ERROR_INVALID_LOCK);
            assert(afsplus_aros_packet_waiting(context) == 1);
            afsplus_aros_packet_elapsed(context, 10);
            assert(completed_count == 5 && completed[4] == &second);

            /* The table is bounded: the seventeenth waiter answers at once.
             * The others expire together, oldest first. */
            completed_count = 0;
            for (index = 0; index < 17; index++)
            {
                many[index] = first;
                many[index].dp_Arg5 = 5;
                assert(afsplus_aros_packet_process(context, &many[index])
                    == (index < 16 ? AFSPLUS_AROS_PACKET_DEFERRED : 0));
            }
            assert(many[16].dp_Res1 == DOSFALSE
                && many[16].dp_Res2 == ERROR_LOCK_TIMEOUT);
            assert(afsplus_aros_packet_waiting(context) == 16);
            afsplus_aros_packet_elapsed(context, 5);
            assert(completed_count == 16
                && afsplus_aros_packet_waiting(context) == 0);
            for (index = 0; index < 16; index++)
                assert(completed[index] == &many[index]
                    && many[index].dp_Res2 == ERROR_LOCK_TIMEOUT);
            stub_record_error = 0;
            stub_free_succeeds = 0;
            completed_count = 0;
            /* "other" was closed in this block. */
            closes++;
        }

        initialize_packet(&packet, ACTION_END);
        packet.dp_Arg1 = from_lock.fh_Arg1;
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == DOSTRUE && close_count == closes + 1);
    }

    /* C2: ACTION_RENAME_DISK: prepare the DOS node, relabel the volume,
     * commit the node. */
    reset_events();
    initialize_packet(&packet, ACTION_RENAME_DISK);
    packet.dp_Arg1 = packet_bstr("Work");
    assert(afsplus_aros_packet_process(context, &packet) == 0);
    assert(packet.dp_Res1 == DOSTRUE && packet.dp_Res2 == 0);
    assert_event(0, 'V', "Work", 0);
    assert(strcmp(relabel_phases, "PC") == 0);
    assert(relabelled_length == 4 && memcmp(relabelled, "Work", 4) == 0);
    /* A label the volume refuses aborts the preparation: the node keeps
     * its name. */
    stub_label_error = ERROR_INVALID_COMPONENT_NAME;
    packet.dp_Arg1 = packet_bstr("a:b");
    assert(afsplus_aros_packet_process(context, &packet) == 0);
    assert(packet.dp_Res1 == DOSFALSE
        && packet.dp_Res2 == ERROR_INVALID_COMPONENT_NAME);
    assert(strcmp(relabel_phases, "PCPA") == 0);
    assert(relabelled_length == 4);
    stub_label_error = 0;
    /* A handler that cannot prepare leaves the volume untouched. */
    stub_relabel_prepare_error = ERROR_OBJECT_IN_USE;
    reset_events();
    packet.dp_Arg1 = packet_bstr("Busy");
    assert(afsplus_aros_packet_process(context, &packet) == 0);
    assert(packet.dp_Res1 == DOSFALSE
        && packet.dp_Res2 == ERROR_OBJECT_IN_USE);
    assert(strcmp(relabel_phases, "PCPAP") == 0 && event_count == 0);
    stub_relabel_prepare_error = 0;

    /* C2: ACTION_WRITE_PROTECT carries the flag and the 32-bit pass key. */
    initialize_packet(&packet, ACTION_WRITE_PROTECT);
    packet.dp_Arg1 = DOSTRUE;
    packet.dp_Arg2 = (SIPTR)UINT32_C(0xC0FFEE42);
    assert(afsplus_aros_packet_process(context, &packet) == 0);
    assert(packet.dp_Res1 == DOSTRUE && packet.dp_Res2 == 0);
    assert(stub_protect == 1 && stub_protect_key == UINT32_C(0xC0FFEE42));
    packet.dp_Arg1 = DOSFALSE;
    packet.dp_Arg2 = 1;
    assert(afsplus_aros_packet_process(context, &packet) == 0);
    assert(packet.dp_Res1 == DOSFALSE
        && packet.dp_Res2 == ERROR_DISK_WRITE_PROTECTED);
    assert(stub_protect == 1);
    packet.dp_Arg2 = (SIPTR)UINT32_C(0xC0FFEE42);
    assert(afsplus_aros_packet_process(context, &packet) == 0);
    assert(packet.dp_Res1 == DOSTRUE && stub_protect == 0);
    /* A revision-8 library has the group and lacks the entry point. */
    {
        struct AfsplusArosPacketContext *older = NULL;

        stub_revision = 8;
        assert(afsplus_aros_packet_create(&config, &older) == 0);
        stub_revision = AFSPLUS_AROS_INTERFACE_REVISION;
        reset_events();
        packet.dp_Arg1 = DOSTRUE;
        assert(afsplus_aros_packet_process(older, &packet) == 0);
        assert(packet.dp_Res1 == DOSFALSE
            && packet.dp_Res2 == ERROR_ACTION_NOT_KNOWN);
        assert(event_count == 0 && stub_protect == 0);
        assert(afsplus_aros_packet_destroy(older) == 0);
    }

    /* C8: notification requests map to watches; fired watches are delivered
     * after the packet that caused them. */
    {
        struct NotifyRequest first;

        memset(&first, 0, sizeof(first));
        memset(&second, 0, sizeof(second));
        first.nr_FullName = (STRPTR)"AFS+:Prefs/settings";
        first.nr_MsgCount = 9;
        second.nr_FullName = (STRPTR)"AFS+:Prefs";
        second.nr_Flags = NRF_NOTIFY_INITIAL;

        reset_events();
        initialize_packet(&packet, ACTION_ADD_NOTIFY);
        packet.dp_Arg1 = (SIPTR)&first;
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == DOSTRUE && packet.dp_Res2 == 0);
        assert(first.nr_Handler == config.handler_port);
        /* The requester's message count is not the handler's to reset. */
        assert(first.nr_MsgCount == 9);
        /* Parent resolved from the volume root, leaf watched by name. */
        assert_event(0, 'L', "Prefs", AFSPLUS_AROS_LOCK_SHARED);
        assert_event(1, 'W', "settings", 0);
        assert(delivered_count == 0);

        assert(afsplus_aros_packet_notify_registered(context, &first) == 1);
        assert(afsplus_aros_packet_notify_registered(context, &second) == 0);
        assert(afsplus_aros_packet_notify_registered(context, NULL) == 0);

        /* The same request twice is refused. */
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == DOSFALSE
            && packet.dp_Res2 == ERROR_OBJECT_IN_USE);

        /* NRF_NOTIFY_INITIAL on an existing object delivers at once. */
        initialize_packet(&packet, ACTION_ADD_NOTIFY);
        packet.dp_Arg1 = (SIPTR)&second;
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == DOSTRUE);
        assert(delivered_count == 1 && delivered[0] == &second);

        /* Watches 500 (first) and 501 (second) fire during an unrelated
         * packet: both requests are delivered once, unknown id 777 to
         * nobody. */
        delivered_count = 0;
        stub_fired[0] = 501;
        stub_fired[1] = 777;
        stub_fired[2] = 500;
        stub_fired_count = 3;
        initialize_packet(&packet, ACTION_IS_FILESYSTEM);
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(delivered_count == 2);
        assert(delivered[0] == &second && delivered[1] == &first);
        /* Nothing fired: nothing delivered. */
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(delivered_count == 2);

        /* Removal stops delivery for that request only. */
        initialize_packet(&packet, ACTION_REMOVE_NOTIFY);
        packet.dp_Arg1 = (SIPTR)&first;
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == DOSTRUE && stub_removed_watches == 1);
        assert(afsplus_aros_packet_notify_registered(context, &first) == 0);
        assert(afsplus_aros_packet_notify_registered(context, &second) == 1);
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(packet.dp_Res1 == DOSFALSE
            && packet.dp_Res2 == ERROR_OBJECT_NOT_FOUND);
        delivered_count = 0;
        stub_fired[0] = 500;
        stub_fired[1] = 501;
        stub_fired_count = 2;
        initialize_packet(&packet, ACTION_IS_FILESYSTEM);
        assert(afsplus_aros_packet_process(context, &packet) == 0);
        assert(delivered_count == 1 && delivered[0] == &second);
        /* "second" stays registered for the ACTION_DIE cases below. */
    }

    /* C1 control: a library without the later groups makes the same packets
     * unknown actions, and no boundary function is reached. */
    {
        struct AfsplusArosPacketContext *old_context = NULL;

        stub_groups = AFSPLUS_AROS_GROUP_BASE;
        assert(afsplus_aros_packet_create(&config, &old_context) == 0);
        reset_events();
        initialize_packet(&packet, ACTION_SET_PROTECT);
        packet.dp_Arg3 = packet_bstr("note");
        assert(afsplus_aros_packet_process(old_context, &packet) == 0);
        assert(packet.dp_Res1 == DOSFALSE
            && packet.dp_Res2 == ERROR_ACTION_NOT_KNOWN);
        initialize_packet(&packet, ACTION_MAKE_LINK);
        packet.dp_Arg2 = packet_bstr("alias");
        packet.dp_Arg3 = (SIPTR)"real";
        packet.dp_Arg4 = LINK_SOFT;
        assert(afsplus_aros_packet_process(old_context, &packet) == 0);
        assert(packet.dp_Res1 == DOSFALSE
            && packet.dp_Res2 == ERROR_ACTION_NOT_KNOWN);
        initialize_packet(&packet, ACTION_SET_COMMENT);
        packet.dp_Arg3 = packet_bstr("note");
        packet.dp_Arg4 = packet_bstr("text");
        assert(afsplus_aros_packet_process(old_context, &packet) == 0);
        assert(packet.dp_Res1 == DOSFALSE
            && packet.dp_Res2 == ERROR_ACTION_NOT_KNOWN);
        /* Packets by type and failures by code, exactly. This context has
         * answered SET_PROTECT, MAKE_LINK and SET_COMMENT, each once and each
         * with ERROR_ACTION_NOT_KNOWN. */
        {
            struct AfsplusExtRequest request;
            struct AfsplusExtPacketCount counts[8];
            struct DosPacket unknown;
            uint32_t index;

            memset(&request, 0, sizeof(request));
            request.magic = AFSPLUS_EXT_MAGIC;
            request.version = AFSPLUS_EXT_VERSION;
            request.header_size = sizeof(request);
            request.operation = AFSPLUS_EXT_PACKET_COUNTS;
            request.buffer = counts;
            request.buffer_size = sizeof(counts);
            initialize_packet(&packet, ACTION_AFSPLUS_EXT);
            packet.dp_Arg1 = (SIPTR)&request;
            memset(counts, 0x7e, sizeof(counts));
            assert(afsplus_aros_packet_process(old_context, &packet) == 0);
            assert(packet.dp_Res1 == DOSTRUE);
            assert(request.output_count == 3 && request.output_value == 3);
            assert(counts[0].key == ACTION_SET_PROTECT
                && counts[0].count == 1 && counts[0].failed == 1);
            assert(counts[1].key == ACTION_MAKE_LINK && counts[1].count == 1);
            assert(counts[2].key == ACTION_SET_COMMENT
                && counts[2].failed == 1);
            assert(counts[3].key == 0x7e7e7e7e);

            request.flags = AFSPLUS_EXT_COUNT_BY_ERROR;
            assert(afsplus_aros_packet_process(old_context, &packet) == 0);
            assert(request.output_count == 1 && request.output_value == 1);
            assert(counts[0].key == ERROR_ACTION_NOT_KNOWN
                && counts[0].count == 3 && counts[0].failed == 3);

            /* The two requests above succeeded and are counted by now; a
             * short buffer gets a prefix and the size of the whole. */
            request.flags = AFSPLUS_EXT_COUNT_BY_ACTION;
            request.buffer_size = 2 * sizeof(counts[0]) + 5;
            memset(counts, 0x7e, sizeof(counts));
            assert(afsplus_aros_packet_process(old_context, &packet) == 0);
            assert(request.output_count == 2 && request.output_value == 4);
            assert(counts[2].key == 0x7e7e7e7e);
            request.buffer_size = sizeof(counts);
            assert(afsplus_aros_packet_process(old_context, &packet) == 0);
            assert(counts[3].key == ACTION_AFSPLUS_EXT
                && counts[3].count == 3 && counts[3].failed == 0);
            request.flags = 2;
            assert(afsplus_aros_packet_process(old_context, &packet) == 0);
            assert(packet.dp_Res2 == ERROR_BAD_NUMBER);

            /* Sixty-four types are counted by name; the rest share one
             * record, which comes last and loses no packet. */
            for (index = 0; index < 70; index++)
            {
                initialize_packet(&unknown, (LONG)(900000 + index));
                assert(afsplus_aros_packet_process(old_context, &unknown)
                    == 0);
                assert(unknown.dp_Res2 == ERROR_ACTION_NOT_KNOWN);
            }
            {
                static struct AfsplusExtPacketCount all[70];

                request.flags = AFSPLUS_EXT_COUNT_BY_ACTION;
                request.buffer = all;
                request.buffer_size = sizeof(all);
                assert(afsplus_aros_packet_process(old_context, &packet)
                    == 0);
                assert(request.output_count == 65
                    && request.output_value == 65);
                assert(all[63].key == 900000 + 59 && all[63].count == 1);
                assert(all[64].key == AFSPLUS_EXT_COUNT_OTHER
                    && all[64].count == 10 && all[64].failed == 10);
                request.flags = AFSPLUS_EXT_COUNT_BY_ERROR;
                assert(afsplus_aros_packet_process(old_context, &packet)
                    == 0);
                assert(request.output_count == 2);
                assert(all[0].key == ERROR_ACTION_NOT_KNOWN
                    && all[0].count == 73);
                assert(all[1].key == ERROR_BAD_NUMBER && all[1].count == 1);
            }
        }

        /* The extension packet itself is known; it reports which groups it
         * can reach, and an operation of an absent group is unknown. */
        {
            struct AfsplusExtRequest request;
            uint8_t byte;

            memset(&request, 0, sizeof(request));
            request.magic = AFSPLUS_EXT_MAGIC;
            request.version = AFSPLUS_EXT_VERSION;
            request.header_size = sizeof(request);
            request.operation = AFSPLUS_EXT_INTERFACE;
            initialize_packet(&packet, ACTION_AFSPLUS_EXT);
            packet.dp_Arg1 = (SIPTR)&request;
            assert(afsplus_aros_packet_process(old_context, &packet) == 0);
            assert(packet.dp_Res1 == DOSTRUE);
            assert(request.output_value == AFSPLUS_AROS_GROUP_BASE);
            /* A handler without a clock cannot stamp a change. That too is
             * not "unknown packet": a client that fell back here would move
             * the file position for a write that cannot succeed either. */
            {
                struct AfsplusArosPacketContext *clockless = NULL;

                stub_groups = UINT64_C(0x7FFF);
                config.now = NULL;
                assert(afsplus_aros_packet_create(&config, &clockless) == 0);
                config.now = packet_now;
                stub_groups = AFSPLUS_AROS_GROUP_BASE;
                request.operation = AFSPLUS_EXT_REPLACE;
                request.name0 = (const uint8_t *)"a";
                request.name_length[0] = 1;
                request.name1 = (const uint8_t *)"b";
                request.name_length[1] = 1;
                reset_events();
                assert(afsplus_aros_packet_process(clockless, &packet) == 0);
                assert(packet.dp_Res1 == DOSFALSE
                    && packet.dp_Res2 == ERROR_NOT_IMPLEMENTED);
                assert(event_count == 0);
                assert(afsplus_aros_packet_destroy(clockless) == 0);
                request.name0 = NULL;
                request.name1 = NULL;
                request.name_length[0] = 0;
                request.name_length[1] = 0;
            }

            /* Never ERROR_ACTION_NOT_KNOWN from inside the transport: that
             * value tells a client the packet itself is unknown. */
            request.operation = AFSPLUS_EXT_INFO_JSON;
            request.buffer = &byte;
            request.buffer_size = 1;
            assert(afsplus_aros_packet_process(old_context, &packet) == 0);
            assert(packet.dp_Res1 == DOSFALSE
                && packet.dp_Res2 == ERROR_NOT_IMPLEMENTED);
        }
        assert(event_count == 0);
        assert(afsplus_aros_packet_destroy(old_context) == 0);

        stub_groups = 0;
        assert(afsplus_aros_packet_create(&config, &old_context)
            == ERROR_BAD_NUMBER);
        stub_groups = UINT64_C(0x7FFF);

        /* A handler shell without a delivery callback cannot notify, so the
         * request is an unknown action and no watch is created. */
        {
            struct NotifyRequest request;

            memset(&request, 0, sizeof(request));
            request.nr_FullName = (STRPTR)"AFS+:x";
            config.notify = NULL;
            config.relabel = NULL;
            config.complete = NULL;
            assert(afsplus_aros_packet_create(&config, &old_context) == 0);
            reset_events();
            initialize_packet(&packet, ACTION_ADD_NOTIFY);
            packet.dp_Arg1 = (SIPTR)&request;
            assert(afsplus_aros_packet_process(old_context, &packet) == 0);
            assert(packet.dp_Res1 == DOSFALSE
                && packet.dp_Res2 == ERROR_ACTION_NOT_KNOWN);
            assert(event_count == 0);
            /* Nor can it rename its DOS node, so the label stays untouched. */
            initialize_packet(&packet, ACTION_RENAME_DISK);
            packet.dp_Arg1 = packet_bstr("Work");
            assert(afsplus_aros_packet_process(old_context, &packet) == 0);
            assert(packet.dp_Res1 == DOSFALSE
                && packet.dp_Res2 == ERROR_ACTION_NOT_KNOWN);
            assert(event_count == 0);
            /* A context destroyed with a waiter hands the packet back before
             * it closes the file the packet names. */
            {
                struct AfsplusArosPacketContext *dying = NULL;
                struct FileHandle held;
                struct DosPacket waiter;
                uint32_t closes_before;

                config.complete = packet_complete;
                assert(afsplus_aros_packet_create(&config, &dying) == 0);
                config.complete = NULL;
                memset(&held, 0, sizeof(held));
                initialize_packet(&packet, ACTION_FINDOUTPUT);
                packet.dp_Arg1 = (SIPTR)MKBADDR(&held);
                packet.dp_Arg3 = packet_bstr("held");
                assert(afsplus_aros_packet_process(dying, &packet) == 0);
                stub_record_error = ERROR_LOCK_COLLISION;
                initialize_packet(&waiter, ACTION_LOCK_RECORD);
                waiter.dp_Arg1 = held.fh_Arg1;
                waiter.dp_Arg3 = 4;
                waiter.dp_Arg4 = REC_SHARED;
                waiter.dp_Arg5 = 50;
                assert(afsplus_aros_packet_process(dying, &waiter)
                    == AFSPLUS_AROS_PACKET_DEFERRED);
                stub_record_error = 0;
                completed_count = 0;
                closes_before = close_count;
                assert(afsplus_aros_packet_destroy(dying) == 0);
                assert(completed_count == 1 && completed[0] == &waiter);
                assert(waiter.dp_Res1 == DOSFALSE
                    && waiter.dp_Res2 == ERROR_DEVICE_NOT_MOUNTED);
                assert(close_count == closes_before + 1);
                completed_count = 0;
            }

            /* Nor can it take a packet back later: a waiting record lock
             * with a timeout still answers at once. */
            {
                struct FileHandle plain;

                memset(&plain, 0, sizeof(plain));
                initialize_packet(&packet, ACTION_FINDOUTPUT);
                packet.dp_Arg1 = (SIPTR)MKBADDR(&plain);
                packet.dp_Arg3 = packet_bstr("plain");
                assert(afsplus_aros_packet_process(old_context, &packet) == 0);
                assert(packet.dp_Res1 == DOSTRUE);
                stub_record_error = ERROR_LOCK_COLLISION;
                initialize_packet(&packet, ACTION_LOCK_RECORD);
                packet.dp_Arg1 = plain.fh_Arg1;
                packet.dp_Arg3 = 4;
                packet.dp_Arg4 = REC_EXCLUSIVE;
                packet.dp_Arg5 = 50;
                assert(afsplus_aros_packet_process(old_context, &packet) == 0);
                assert(packet.dp_Res1 == DOSFALSE
                    && packet.dp_Res2 == ERROR_LOCK_TIMEOUT);
                assert(afsplus_aros_packet_waiting(old_context) == 0);
                stub_record_error = 0;
            }
            assert(afsplus_aros_packet_destroy(old_context) == 0);
            config.notify = packet_notify;
            config.relabel = packet_relabel;
            config.complete = packet_complete;
        }
    }

    initialize_packet(&packet, ACTION_DIE);
    assert(afsplus_aros_packet_process(context, &packet) == 0);
    assert(packet.dp_Res1 == DOSFALSE && packet.dp_Res2 == ERROR_OBJECT_IN_USE);

    initialize_packet(&packet, ACTION_INHIBIT);
    packet.dp_Arg1 = DOSTRUE;
    assert(afsplus_aros_packet_process(context, &packet) == 0);
    assert(packet.dp_Res1 == DOSFALSE && packet.dp_Res2 == ERROR_OBJECT_IN_USE);
    assert(flush_count == 0);

    initialize_packet(&packet, ACTION_FREE_LOCK);
    packet.dp_Arg1 = (SIPTR)located;
    assert(afsplus_aros_packet_process(context, &packet) == 0);
    assert(packet.dp_Res1 == DOSTRUE && packet.dp_Res2 == 0);
    initialize_packet(&packet, ACTION_FREE_LOCK);
    packet.dp_Arg1 = (SIPTR)created;
    assert(afsplus_aros_packet_process(context, &packet) == 0);
    assert(packet.dp_Res1 == DOSTRUE && packet.dp_Res2 == 0);
    initialize_packet(&packet, ACTION_FREE_LOCK);
    packet.dp_Arg1 = (SIPTR)root;
    assert(afsplus_aros_packet_process(context, &packet) == 0);
    assert(packet.dp_Res1 == DOSTRUE && packet.dp_Res2 == 0);

    /* No lock or file is open, but "second" is registered and its nr_Handler
     * names this port: the handler must not die under it. */
    initialize_packet(&packet, ACTION_DIE);
    assert(afsplus_aros_packet_process(context, &packet) == 0);
    assert(packet.dp_Res1 == DOSFALSE && packet.dp_Res2 == ERROR_OBJECT_IN_USE);
    assert(afsplus_aros_packet_should_quit(context) == 0);
    assert(second.nr_Handler == config.handler_port);
    initialize_packet(&packet, ACTION_REMOVE_NOTIFY);
    packet.dp_Arg1 = (SIPTR)&second;
    assert(afsplus_aros_packet_process(context, &packet) == 0);
    assert(packet.dp_Res1 == DOSTRUE && stub_removed_watches == 2);

    /* A context destroyed with a live registration detaches the request, so
     * a later EndNotify sends nothing to the vanished port. */
    {
        struct AfsplusArosPacketContext *doomed = NULL;
        struct NotifyRequest orphan;

        memset(&orphan, 0, sizeof(orphan));
        orphan.nr_FullName = (STRPTR)"AFS+:orphan";
        assert(afsplus_aros_packet_create(&config, &doomed) == 0);
        initialize_packet(&packet, ACTION_ADD_NOTIFY);
        packet.dp_Arg1 = (SIPTR)&orphan;
        assert(afsplus_aros_packet_process(doomed, &packet) == 0);
        assert(orphan.nr_Handler == config.handler_port);
        assert(afsplus_aros_packet_destroy(doomed) == 0);
        assert(orphan.nr_Handler == NULL);
        assert(stub_removed_watches == 3);
    }

    initialize_packet(&packet, ACTION_INHIBIT);
    packet.dp_Arg1 = DOSTRUE;
    assert(afsplus_aros_packet_process(context, &packet) == 0);
    assert(packet.dp_Res1 == DOSTRUE && packet.dp_Res2 == 0);
    assert(flush_count == 1);

    initialize_packet(&packet, ACTION_INHIBIT);
    packet.dp_Arg1 = DOSTRUE;
    assert(afsplus_aros_packet_process(context, &packet) == 0);
    assert(packet.dp_Res1 == DOSTRUE && packet.dp_Res2 == 0);
    assert(flush_count == 1);

    initialize_packet(&packet, ACTION_INHIBIT);
    packet.dp_Arg1 = DOSFALSE;
    assert(afsplus_aros_packet_process(context, &packet) == 0);
    assert(packet.dp_Res1 == DOSTRUE && packet.dp_Res2 == 0);

    initialize_packet(&packet, ACTION_INHIBIT);
    packet.dp_Arg1 = DOSTRUE;
    assert(afsplus_aros_packet_process(context, &packet) == 0);
    assert(packet.dp_Res1 == DOSTRUE && packet.dp_Res2 == 0);
    assert(flush_count == 2);

    initialize_packet(&packet, ACTION_DIE);
    assert(afsplus_aros_packet_process(context, &packet) == 0);
    assert(packet.dp_Res1 == DOSTRUE && packet.dp_Res2 == 0);
    assert(afsplus_aros_packet_should_quit(context) == 1);
    assert(flush_count == 3);

    assert(afsplus_aros_packet_destroy(context) == 0);
    assert(stub_removed_watches == 3);
    puts("afsplus packet stub: PASS");
    return 0;
}
