/* Copyright (C) 2026 John KNIPPER <code@jkn.me>
 * Licensed under the AROS Public License (APL) Version 1.1. */
#define main snapshot_existing_matrix
#include "mmap_snapshot_ffi_packet.c"
#undef main
static void writeback(struct AfsplusArosPacketContext *context,
    struct FSV2_WritebackRequest *request,int32_t error)
{
    struct DosPacket packet={0};
    packet.dp_Type=ACTION_FILESYSTEM_PAGING_V2;
    packet.dp_Arg1=(SIPTR)request;
    assert(afsplus_aros_packet_process(context,&packet)==0);
    if (packet.dp_Res2!=error) { printf("writeback expected=%ld got=%ld\n",(long)error,(long)packet.dp_Res2);fflush(NULL); }
    assert(packet.dp_Res2==error);
    assert(packet.dp_Res1==(error ? DOSFALSE : DOSTRUE));
}
int main(void)
{
    uint8_t uuid[16]={0x79},before[3*4096],after[sizeof(before)],read[sizeof(before)];
    struct AfsplusArosDevice device={0};
    struct AfsplusArosMountConfig mount={0};
    struct AfsplusArosPacketConfig config={0};
    struct AfsplusAros *fs=NULL;
    struct AfsplusArosPacketContext *context=NULL;
    struct FileHandle file={0};
    struct DosPacket packet={0};
    uint64_t id=0;uint32_t count=0;
    memset(before,0x31,sizeof(before));memset(after,0xa6,sizeof(after));
    device.abi_version=AFSPLUS_AROS_ABI_VERSION;device.struct_size=sizeof(device);
    device.context=calloc(BLOCKS,BLOCK_SIZE);assert(device.context);
    device.block_size=BLOCK_SIZE;device.total_blocks=BLOCKS;
    device.read_block=read_block;device.write_block=write_block;device.flush=flush;
    assert(!afsplus_aros_format(&device,AFSPLUS_AROS_ENCODING_UTF8,
        (const uint8_t *)"Writeback",9,uuid,1,0));
    mount.abi_version=AFSPLUS_AROS_ABI_VERSION;mount.struct_size=sizeof(mount);
    mount.max_file_handles=32;mount.max_locks=32;mount.max_file_info_name_bytes=107;
    assert(!afsplus_aros_mount(&device,&mount,&fs));
    assert(!afsplus_aros_open(fs,0,(const uint8_t *)"file",4,
        AFSPLUS_AROS_OPEN_NEW_FILE,1,0,&id));
    assert(!afsplus_aros_write_at(fs,id,0,before,sizeof(before),2,0,&count));
    assert(!afsplus_aros_close(fs,id));
    config.abi_version=AFSPLUS_AROS_PACKET_ABI_VERSION;config.struct_size=sizeof(config);
    config.filesystem=fs;config.handler_port=(void *)(uintptr_t)0x2000;
    config.allocate=allocate;config.free=release;config.now=now;
    assert(!afsplus_aros_packet_create(&config,&context));
    packet.dp_Type=ACTION_FINDUPDATE;packet.dp_Arg1=(SIPTR)MKBADDR(&file);
    packet.dp_Arg3=(SIPTR)"file";send(context,&packet);
    struct FSV2_WritebackRequest request={.struct_size=sizeof(request),
        .version=FSV2_PAGING_VERSION,.operation=FSV2_PAGING_WRITEBACK_FROZEN,
        .flags=FSV2_WRITEBACK_SYNC,.source=file.fh_Arg1,.buffer=after,
        .length=sizeof(after),.output_count=UINT64_MAX};
    struct FSV2_WritebackRequest good=request;
    /* Every rejected request preserves the caller's output field. */
    request.flags=0;writeback(context,&request,115);assert(request.output_count==UINT64_MAX);
    request=good;request.reserved=1;writeback(context,&request,115);
    request=good;request.reserved2=1;writeback(context,&request,115);
    request=good;request.buffer=NULL;writeback(context,&request,115);
    request=good;request.length=(uint64_t)UINT32_MAX+1;writeback(context,&request,115);
    request=good;request.offset=UINT64_MAX;writeback(context,&request,115);
    request=good;request.source=0;writeback(context,&request,211);
    request=good;request.struct_size=8;writeback(context,&request,236);
    request=good;request.offset=1;writeback(context,&request,210);
    assert(request.output_count==UINT64_MAX);
    request=good;writeback(context,&request,0);assert(request.output_count==sizeof(after));
    /* Ordinary Read through the same open cookie begins at position zero. */
    memset(&packet,0,sizeof(packet));packet.dp_Type=ACTION_READ;
    packet.dp_Arg1=file.fh_Arg1;packet.dp_Arg2=(SIPTR)read;packet.dp_Arg3=sizeof(read);
    send(context,&packet);assert(packet.dp_Res1==sizeof(read) && !memcmp(read,after,sizeof(read)));
    memset(&packet,0,sizeof(packet));packet.dp_Type=ACTION_END;packet.dp_Arg1=file.fh_Arg1;
    send(context,&packet);
    memset(&file,0,sizeof(file));memset(&packet,0,sizeof(packet));
    packet.dp_Type=ACTION_FINDINPUT;packet.dp_Arg1=(SIPTR)MKBADDR(&file);
    packet.dp_Arg3=(SIPTR)"file";send(context,&packet);
    request=good;request.source=file.fh_Arg1;
    /* DOS MODE_OLDFILE is writable on a writable volume, as on FFS/SFS. */
    writeback(context,&request,0);assert(request.output_count==sizeof(after));
    memset(&packet,0,sizeof(packet));packet.dp_Type=ACTION_END;packet.dp_Arg1=file.fh_Arg1;
    send(context,&packet);assert(!afsplus_aros_packet_destroy(context) && !live);
    assert(!afsplus_aros_unmount(fs));assert(!afsplus_aros_mount(&device,&mount,&fs));
    assert(!afsplus_aros_open(fs,0,(const uint8_t *)"file",4,
        AFSPLUS_AROS_OPEN_OLD_FILE,101,0,&id));
    assert(!afsplus_aros_read_at(fs,id,0,read,sizeof(read),&count));
    assert(count==sizeof(read) && !memcmp(read,after,sizeof(read)));
    assert(!afsplus_aros_close(fs,id));assert(!afsplus_aros_unmount(fs));free(device.context);
    assert(!snapshot_existing_matrix());
    puts("native packet + actual Rust writeback PASS durable positioned, input guards, snapshots retained; no shared coherence gate");
    return 0;
}
