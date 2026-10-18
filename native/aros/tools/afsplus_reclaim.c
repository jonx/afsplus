/* SPDX-License-Identifier: BSD-2-Clause */
/* Copyright (c) 2026 John Knipper */
/* One bounded ADR-133 maintenance step; state is saved outside the volume. */
#include <dos/dos.h>
#include <dos/dosextens.h>
#include <proto/dos.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "../client/afsplus_client.h"

static int budget(const char *s, uint32_t max, uint32_t *out)
{
    uint32_t n=0;
    if(!*s)return 0;
    for(;*s;s++) {if(*s<'0'||*s>'9'||n>(max-(uint32_t)(*s-'0'))/10)return 0;n=n*10+(uint32_t)(*s-'0');}
    if(!n||n>max)return 0;*out=n;return 1;
}
int main(int argc,char **argv)
{
    struct DevProc *device=NULL;
    struct AfsplusArosRestoreRecovery *r=NULL;
    BPTR scope=BNULL,in=BNULL,out=BNULL,cursor_parent=BNULL,old_dir=BNULL;
    const char *cursor=argc>=4?argv[3]:"RAM:AFSPlusReclaim.cursor";
    char *temporary=NULL,*parent_path=NULL,*cursor_name=NULL;
    LONG error=0,n;
    int result=RETURN_FAIL,owned_temp=0,changed_dir=0;
    if(argc!=3&&argc!=4&&argc!=7) {
        fprintf(stderr,"usage: AFSPlusReclaim <volume> <directory> [cursor-file [scan work depth]]\n"
            "One bounded private-stage maintenance step; repeat while complete=0.\n"
            "Defaults: RAM:AFSPlusReclaim.cursor, 64 entries, 8 work steps, depth64.\n"
            "Keep cursor outside repaired volume. On stale cursor remove it and restart a quiescent subtree.\n");return RETURN_ERROR;
    }
    r=calloc(1,sizeof(*r));if(!r){fprintf(stderr,"AFSPlusReclaim: no memory for bounded request.\n");return RETURN_FAIL;}
    r->struct_size=sizeof(*r);r->scan_entries=64;r->work_steps=8;r->max_depth=64;
    if(argc==7&&(!budget(argv[4],64,&r->scan_entries)||!budget(argv[5],8,&r->work_steps)||!budget(argv[6],64,&r->max_depth))){error=ERROR_BAD_NUMBER;goto done;}
    device=GetDeviceProc((CONST_STRPTR)argv[1],NULL);
    if(!device){error=IoErr();goto done;}
    scope=Lock((CONST_STRPTR)argv[2],SHARED_LOCK);
    if(!scope){error=IoErr();goto done;}
    if(afsplus_client_lock_port(scope)!=device->dvp_Port){error=ERROR_RENAME_ACROSS_DEVICES;goto done;}
    parent_path=malloc(strlen(cursor)+1);cursor_name=malloc(strlen(cursor)+1);
    if(!parent_path||!cursor_name){error=ERROR_NO_FREE_STORE;goto done;}
    strcpy(parent_path,cursor);strcpy(cursor_name,(const char *)FilePart((CONST_STRPTR)cursor));
    if(!*cursor_name||!strcmp(cursor_name,".")||!strcmp(cursor_name,"..")){error=ERROR_BAD_NUMBER;goto done;}
    *(char *)PathPart((CONST_STRPTR)parent_path)=0;
    cursor_parent=Lock((CONST_STRPTR)parent_path,SHARED_LOCK);
    if(!cursor_parent){error=IoErr();goto done;}
    if(afsplus_client_lock_port(cursor_parent)==device->dvp_Port){error=ERROR_RENAME_ACROSS_DEVICES;goto done;}
    /* Pin the resolved parent, including symlinks/assigns, and perform every
     * cursor operation relative to that lock. Re-resolving the original path
     * would let an assign or link change redirect writes after admission. */
    old_dir=CurrentDir(cursor_parent);changed_dir=1;cursor=cursor_name;
    in=Open((CONST_STRPTR)cursor,MODE_OLDFILE);
    if(in) {
        uint8_t extra;
        n=Read(in,r->cursor,sizeof(r->cursor));
        if(n<=0||Read(in,&extra,1)!=0){error=ERROR_BAD_NUMBER;goto done;}
        r->cursor_size=(uint32_t)n;
        if(!Close(in)){error=IoErr();in=BNULL;goto done;}in=BNULL;
    } else if(IoErr()!=ERROR_OBJECT_NOT_FOUND){error=IoErr();goto done;}
    error=afsplus_client_restore_reclaim(device->dvp_Port,scope,r);
    if(error)goto done;
    printf("AFSPlusReclaim: scanned=%lu removed=%lu advanced=%lu blocked=%lu blocked-id=%llu complete=%lu\n",
        (unsigned long)r->scanned,(unsigned long)r->removed,(unsigned long)r->advanced,
        (unsigned long)r->blocked,(unsigned long long)r->blocked_object,
        (unsigned long)((r->flags&AFSPLUS_RESTORE_RECOVERY_DONE)!=0));
    if(r->flags&AFSPLUS_RESTORE_RECOVERY_DONE) {
        if(!DeleteFile((CONST_STRPTR)cursor)&&IoErr()!=ERROR_OBJECT_NOT_FOUND){error=IoErr();goto done;}
        result=r->blocked?RETURN_WARN:RETURN_OK;
        if(r->blocked)fprintf(stderr,"Partial: malformed or inconsistent stages remain private; retain report for repair.\n");
    } else {
        /* Exclusive staging avoids overwriting an unrelated sibling. After
         * Flush/Close, an interrupted Delete/Rename can lose only the cursor:
         * the next invocation safely rescans durable stage progress. */
        size_t size=strlen(cursor)+32;
        uint32_t nonce;
        temporary=malloc(size);if(!temporary){error=ERROR_NO_FREE_STORE;goto done;}
        for(nonce=0;nonce<256;nonce++) {
            BPTR lock;
            snprintf(temporary,size,"%s.reclaim-%02lx",cursor,(unsigned long)nonce);
            lock=CreateDir((CONST_STRPTR)temporary);
            if(lock){UnLock(lock);owned_temp=1;break;}
            if(IoErr()!=ERROR_OBJECT_EXISTS){error=IoErr();goto done;}
        }
        if(!owned_temp){error=ERROR_OBJECT_EXISTS;goto done;}
        {
            size_t base=strlen(temporary);
            strcat(temporary,"/cursor");
            out=Open((CONST_STRPTR)temporary,MODE_NEWFILE);
            if(!out||Write(out,r->cursor,r->cursor_size)!=(LONG)r->cursor_size||!Flush(out)){error=IoErr();goto done;}
            if(!Close(out)){error=IoErr();out=BNULL;goto done;}out=BNULL;
            if(!DeleteFile((CONST_STRPTR)cursor)&&IoErr()!=ERROR_OBJECT_NOT_FOUND){error=IoErr();goto done;}
            if(!Rename((CONST_STRPTR)temporary,(CONST_STRPTR)cursor)){error=IoErr();goto done;}
            temporary[base]=0;
            if(!DeleteFile((CONST_STRPTR)temporary)){error=IoErr();goto done;}owned_temp=0;
        }
        result=RETURN_WARN;
    }
done:
    if(in)Close(in);if(out)Close(out);
    if(owned_temp&&temporary) {
        char *slash=strrchr(temporary,'/');
        if(slash&&strcmp(slash,"/cursor")==0){DeleteFile((CONST_STRPTR)temporary);*slash=0;}
        DeleteFile((CONST_STRPTR)temporary);
    }
    if(error) {
        fprintf(stderr,"AFSPlusReclaim: DOS error %ld; no published object is selected.\n",(long)error);
        if(error==ERROR_INVALID_LOCK && r && r->cursor_size)
            fprintf(stderr,"Cursor or directory scope is stale. Remove the saved cursor and restart a quiescent subtree.\n");
        if(error==ERROR_OBJECT_IN_USE)
            fprintf(stderr,"Quiesce writers and sync pending data before retrying maintenance.\n");
    }
    if(changed_dir)CurrentDir(old_dir);
    if(cursor_parent)UnLock(cursor_parent);if(scope)UnLock(scope);if(device)FreeDeviceProc(device);
    free(parent_path);free(cursor_name);free(temporary);free(r);return result;
}
