/* SPDX-License-Identifier: BSD-2-Clause */
/* Copyright (c) 2026 John Knipper */
/* Bounded ADR-133 maintenance; the resumable cursor stays in memory. */
#include <dos/dos.h>
#include <dos/dosextens.h>
#include <proto/dos.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "../client/afsplus_client.h"

/* A write elsewhere on the volume makes the cursor stale; a fresh scan is
 * safe (ADR-133), so a live volume restarts a bounded number of times. */
#define RECLAIM_RESTARTS_MAX 8

static int budget(const char *s, uint32_t max, uint32_t *out)
{
    uint32_t n=0;
    if(!*s)return 0;
    for(;*s;s++) {
        uint32_t digit;
        if(*s<'0'||*s>'9')return 0;
        digit=(uint32_t)(*s-'0');
        if(n>max/10 || (n==max/10 && digit>max%10))return 0;
        n=n*10+digit;
    }
    if(!n)return 0;*out=n;return 1;
}
int main(int argc,char **argv)
{
    struct DevProc *device=NULL;
    struct AfsplusArosRestoreRecovery *r=NULL;
    BPTR scope=BNULL;
    intptr_t error=0;
    uint32_t calls=0,max_calls=4096,blocked=0,restarts=0;
    uint64_t scanned=0,removed=0,advanced=0,blocked_object=0;
    int result=RETURN_FAIL,complete=0,interrupted=0;
    if(argc!=3&&argc!=6&&argc!=7) {
        fprintf(stderr,"usage: AFSPlusReclaim <volume> <directory> [scan work depth [maxcalls]]\n"
            "Defaults: 64 entries, 8 work steps, depth64, 4096 calls.\n"
            "Cursor stays in memory; Ctrl-C or call exhaustion returns partial progress.\n"
            "A later invocation safely rescans durable stages; quiesce the selected subtree.\n");return RETURN_ERROR;
    }
    r=calloc(1,sizeof(*r));
    if(!r){fprintf(stderr,"AFSPlusReclaim: no memory for bounded request.\n");return RETURN_FAIL;}
    r->struct_size=sizeof(*r);r->scan_entries=64;r->work_steps=8;r->max_depth=64;
    if((argc>=6&&(!budget(argv[3],64,&r->scan_entries)||!budget(argv[4],8,&r->work_steps)||!budget(argv[5],64,&r->max_depth)))
        ||(argc==7&&!budget(argv[6],UINT32_MAX,&max_calls))) {
        fprintf(stderr,"AFSPlusReclaim: scan 1-64, work 1-8, depth 1-64 and maxcalls 1-4294967295, in decimal.\n");
        free(r);return RETURN_ERROR;
    }
    device=GetDeviceProc((CONST_STRPTR)argv[1],NULL);
    if(!device){error=IoErr();goto done;}
    scope=Lock((CONST_STRPTR)argv[2],SHARED_LOCK);
    if(!scope){error=IoErr();goto done;}
    if(afsplus_client_lock_port(scope)!=device->dvp_Port){error=ERROR_RENAME_ACROSS_DEVICES;goto done;}
    while(calls<max_calls) {
        if(CheckSignal(SIGBREAKF_CTRL_C)) {interrupted=1;break;}
        calls++;
        error=afsplus_client_restore_reclaim(device->dvp_Port,scope,r);
        if(error==ERROR_INVALID_LOCK&&r->cursor_size&&restarts<RECLAIM_RESTARTS_MAX) {
            restarts++;r->cursor_size=0;r->flags=0;memset(r->cursor,0,sizeof(r->cursor));error=0;continue;
        }
        if(error)break;
        scanned+=r->scanned;removed+=r->removed;advanced+=r->advanced;
        blocked=r->blocked;
        if(r->blocked_object)blocked_object=r->blocked_object;
        if(r->flags&AFSPLUS_RESTORE_RECOVERY_DONE) {complete=1;break;}
    }
    /* Callers match the line's end: new fields go after calls=. */
    printf("AFSPlusReclaim: calls=%lu restarts=%lu scanned=%llu removed=%llu advanced=%llu blocked=%lu blocked-id=%llu complete=%lu\n",
        (unsigned long)calls,(unsigned long)restarts,(unsigned long long)scanned,(unsigned long long)removed,
        (unsigned long long)advanced,(unsigned long)blocked,(unsigned long long)blocked_object,
        (unsigned long)complete);
    if(!error) {
        result=complete&&!blocked?RETURN_OK:RETURN_WARN;
        if(interrupted)fprintf(stderr,"Partial: interrupted; durable progress is retained. A later invocation rescans.\n");
        else if(!complete)fprintf(stderr,"Partial: call budget exhausted; durable progress is retained. A later invocation rescans.\n");
        else if(blocked)fprintf(stderr,"Partial: malformed or inconsistent stages remain private; retain report for repair.\n");
    }
done:
    if(error) {
        if(calls)fprintf(stderr,"AFSPlusReclaim: DOS error %ld; durable private-stage progress may have advanced.\n",(long)error);
        else fprintf(stderr,"AFSPlusReclaim: %s: DOS error %ld; nothing was changed.\n",argv[2],(long)error);
        if(error==ERROR_INVALID_LOCK&&restarts>=RECLAIM_RESTARTS_MAX)
            fprintf(stderr,"The volume kept changing: %lu fresh scans went stale. Quiesce writers in the subtree and retry.\n",(unsigned long)restarts);
        else if(error==ERROR_INVALID_LOCK)
            fprintf(stderr,"Directory scope or generation changed. Restart a quiescent subtree; remount after uncertain storage I/O.\n");
        if(error==ERROR_OBJECT_IN_USE)
            fprintf(stderr,"Quiesce writers and sync pending data before retrying maintenance.\n");
    }
    if(scope)UnLock(scope);if(device)FreeDeviceProc(device);free(r);return result;
}
