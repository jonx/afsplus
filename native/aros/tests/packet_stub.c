/* SPDX-License-Identifier: BSD-2-Clause */

#include "afsplus_packet.h"

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
static uint64_t stub_groups = UINT64_C(0xF);
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
    return examine_common(output, name, name_capacity);
}

int32_t afsplus_aros_rewind_directory(struct AfsplusAros *filesystem,
    uint64_t lock)
{
    assert(filesystem == STUB_FILESYSTEM);
    assert(lock != 0);
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
    output->interface_revision = AFSPLUS_AROS_INTERFACE_REVISION;
    output->groups = stub_groups;
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
        assert(event_count == 0);
        assert(afsplus_aros_packet_destroy(old_context) == 0);

        stub_groups = 0;
        assert(afsplus_aros_packet_create(&config, &old_context)
            == ERROR_BAD_NUMBER);
        stub_groups = UINT64_C(0xF);
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
    puts("afsplus packet stub: PASS");
    return 0;
}
