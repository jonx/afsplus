/* SPDX-License-Identifier: BSD-2-Clause */
#include "libafsplus_reader.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <limits.h>
struct device { FILE *file; unsigned long reads; };
static void require(int ok, const char *message)
{
    if (!ok) { fprintf(stderr, "reservation probe: %s\n", message); exit(1); }
}
static int read_blocks(void *context, uint64_t first, uint32_t count, void *out)
{
    struct device *dev = (struct device *)context;
    if (count != 1u || first > (uint64_t)LONG_MAX / 4096u ||
        fseek(dev->file, (long)(first * 4096u), SEEK_SET) != 0 ||
        fread(out, 1, 4096u, dev->file) != 4096u) { return -1; }
    dev->reads++;
    return 0;
}
int main(int argc, char **argv)
{
    struct device dev;
    struct afspr_block_ops ops;
    struct afspr_probe_result volume;
    struct afspr_object root, object;
    struct afspr_directory_entry entry;
    struct afspr_diagnostic diagnostic;
    uint8_t memory[AFSPR_TREE_SCRATCH_SIZE], data[257], name[32];
    struct afspr_scratch scratch = {memory, sizeof(memory)};
    struct afspr_scratch small = {memory, AFSPR_MIN_SCRATCH_SIZE - 1u};
    uint64_t offset = 0;
    size_t count, i;
    unsigned long before;
    int status, initialized;
    require(argc == 3, "usage: reservation_probe IMAGE before|after|fallback");
    initialized = strcmp(argv[2], "after") == 0;
    require(initialized || strcmp(argv[2], "before") == 0 || strcmp(argv[2], "fallback") == 0, "bad mode");
    dev.file = fopen(argv[1], "rb"); dev.reads = 0;
    require(dev.file != NULL, "open image");
    memset(&ops,0,sizeof(ops));
    ops.abi_version = AFSPR_ABI_VERSION; ops.struct_size = (uint32_t)sizeof(ops);
    ops.ctx = &dev; ops.read_blocks = read_blocks; ops.block_count = 256; ops.block_size = 4096;
    status = afspr_probe(&ops,&scratch,&volume,sizeof(volume));
    require(status == AFSPR_OK, afspr_status_string(status));
    require((strcmp(argv[2],"fallback") == 0) ? volume.valid_checkpoint_mask != 3u : volume.valid_checkpoint_mask == 3u, "checkpoint mask");
    status = afspr_lookup_object(&ops,&scratch,&volume,1,&root,sizeof(root),&diagnostic,sizeof(diagnostic));
    require(status == AFSPR_OK, "root lookup");
    status = afspr_directory_entry_at(&ops,&scratch,&volume,&root,0,name,sizeof(name),&entry,sizeof(entry),NULL,&diagnostic,sizeof(diagnostic));
    require(status == AFSPR_OK && entry.name_len == 8u && memcmp(name,"reserved",8u) == 0, "reserved entry");
    scratch.size = AFSPR_MIN_SCRATCH_SIZE;
    before = dev.reads;
    status = afspr_lookup_object(&ops,&small,&volume,entry.object_id,&object,sizeof(object),&diagnostic,sizeof(diagnostic));
    require(status == AFSPR_ERR_SCRATCH_TOO_SMALL && dev.reads == before, "small workspace must refuse before IO");
    status = afspr_lookup_object(&ops,&scratch,&volume,entry.object_id,&object,sizeof(object),&diagnostic,sizeof(diagnostic));
    require(status == AFSPR_OK, "file lookup");
    while (offset < 16384u) {
        memset(data,0xa5,sizeof(data));
        status = afspr_read_file(&ops,&scratch,&volume,&object,offset,data,sizeof(data),&count,&diagnostic,sizeof(diagnostic));
        require(status == AFSPR_OK && count > 0u && count <= sizeof(data), "bounded read");
        require((uint64_t)count <= 16384u-offset, "read beyond EOF");
        for (i=0; i<count; i++) {
            uint64_t position = offset + (uint64_t)i;
            uint8_t expected = initialized && position >= 7u && position < 5007u ? 0x5au : 0u;
            require(data[i] == expected, "logical byte mismatch");
        }
        offset += (uint64_t)count;
    }
    status = afspr_read_file(&ops,&scratch,&volume,&object,offset,data,sizeof(data),&count,&diagnostic,sizeof(diagnostic));
    require(status == AFSPR_OK && count == 0u, "EOF");
    require(fclose(dev.file) == 0, "close image");
    printf("reservation_c mode=%s bytes=16384 peak_scratch=%lu lookup_read_scratch=4096 chunk=%lu reads=%lu result=PASS\n",argv[2],(unsigned long)sizeof(memory),(unsigned long)sizeof(data),dev.reads);
    return 0;
}
