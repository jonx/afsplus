/* Copyright (C) 2026 John KNIPPER <code@jkn.me>
 * Licensed under the AROS Public License (APL) Version 1.1. */
#define main existing_packet_matrix
#include "packet_stub.c"
#undef main
#include "filesystem_paging_v2.h"
int main(void)
{
    for (unsigned variant=0;variant<3;++variant) {
        struct AfsplusArosPacketConfig config={0};
        struct AfsplusArosPacketContext *context=NULL;
        struct FSV2_WritebackRequest request={.struct_size=sizeof(request),
            .version=FSV2_PAGING_VERSION,.operation=FSV2_PAGING_WRITEBACK_FROZEN,
            .flags=FSV2_WRITEBACK_SYNC,.output_count=UINT64_MAX};
        struct DosPacket packet={0};
        stub_revision=variant==0 ? 22 : AFSPLUS_AROS_INTERFACE_REVISION;
        stub_groups=variant==1 ? AFSPLUS_AROS_GROUP_BASE : UINT64_C(0x1FFFF);
        config.abi_version=AFSPLUS_AROS_PACKET_ABI_VERSION;config.struct_size=sizeof(config);
        config.filesystem=STUB_FILESYSTEM;config.handler_port=(void *)(uintptr_t)0x2000;
        config.allocate=packet_allocate;config.free=packet_free;config.now=packet_now;
        assert(!afsplus_aros_packet_create(&config,&context));
        packet.dp_Type=ACTION_FILESYSTEM_PAGING_V2;packet.dp_Arg1=(SIPTR)&request;
        assert(!afsplus_aros_packet_process(context,&packet));
        assert(packet.dp_Res1==DOSFALSE && request.output_count==UINT64_MAX);
        assert(packet.dp_Res2==(variant==0 ? ERROR_NOT_IMPLEMENTED :
            variant==1 ? ERROR_ACTION_NOT_KNOWN : ERROR_INVALID_LOCK));
        assert(!afsplus_aros_packet_destroy(context) && !paging_live);
    }
    puts("writeback endpoint PASS revision/group/cookie refusal, count preserved");
    return 0;
}
