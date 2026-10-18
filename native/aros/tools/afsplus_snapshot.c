/* SPDX-License-Identifier: BSD-2-Clause */
/* Copyright (c) 2026 John Knipper */
/* AFSPlusSnapshot DEVICE CREATE|LIST|DELETE id|VIEW id [relative-path]. */
#include <dos/dos.h>
#include <dos/dosextens.h>
#include <proto/dos.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#include "../client/afsplus_client.h"

static void number(uint64_t n)
{ char text[21]; snprintf(text,sizeof(text),"%llu",(unsigned long long)n); PutStr(text); }
static LONG view(struct MsgPort *port,uint64_t id,char *path)
{
    uint64_t reader=0,object=1,dir=0,offset=0;
    struct AfsplusArosStat stat;
    uint8_t buffer[4096]; uint32_t count=0,eof=0;
    LONG error=afsplus_client_snapshot_open(port,id,&reader);
    if (error) return error;
    if (path != NULL && *path)
    {
        char *component=path;
        while (*component && !error)
        {
            char *end=strchr(component,'/');
            if (end) *end=0;
            error=afsplus_client_snapshot_lookup(port,reader,object,
                (CONST_STRPTR)component,&object);
            if (!end) break;
            component=end+1;
        }
    }
    if (!error) error=afsplus_client_snapshot_stat(port,reader,object,&stat);
    if (!error && stat.kind==AFSPLUS_AROS_KIND_DIRECTORY)
    {
        error=afsplus_client_snapshot_dir_open(port,reader,object,&dir);
        while (!error && !eof)
        {
            uint32_t i,at=0;
            error=afsplus_client_snapshot_dir_read(port,dir,buffer,sizeof(buffer),8,&count,&eof);
            for (i=0; !error && i<count; i++)
            {
                struct AfsplusArosDirEntry record;
                if (at+sizeof(record)>sizeof(buffer)) { error=ERROR_BAD_NUMBER; break; }
                memcpy(&record,buffer+at,sizeof(record));
                if (record.record_length<sizeof(record)+record.name_length
                    || record.record_length>sizeof(buffer)-at) { error=ERROR_BAD_NUMBER; break; }
                if (Write(Output(),buffer+at+sizeof(record),(LONG)record.name_length)!=(LONG)record.name_length
                    || Write(Output(),"\n",1)!=1) { error=(LONG)IoErr(); if(!error)error=ERROR_DISK_FULL; }
                at+=record.record_length;
            }
        }
        if (dir) { LONG close_error=afsplus_client_snapshot_dir_close(port,dir); if(!error)error=close_error; }
    }
    else if (!error && stat.kind==AFSPLUS_AROS_KIND_FILE)
    {
        do {
            error=afsplus_client_snapshot_read(port,reader,object,offset,buffer,sizeof(buffer),&count);
            if (!error && count && Write(Output(),buffer,(LONG)count)!=(LONG)count)
            { error=(LONG)IoErr(); if(!error)error=ERROR_DISK_FULL; }
            offset+=count;
        } while (!error && count);
    }
    else if (!error) error=ERROR_OBJECT_WRONG_TYPE;
    { LONG close_error=afsplus_client_snapshot_close(port,reader); if(!error)error=close_error; }
    return error;
}
int main(int argc,char **argv)
{
    BPTR lock; struct MsgPort *port; LONG error=0;
    uint64_t id=0; char *end=NULL;
    if (argc<3 || argc>5) { PutStr("AFSPlusSnapshot DEVICE CREATE|LIST|DELETE id|VIEW id [path]\n"); return RETURN_ERROR; }
    if (argc>=4) { id=strtoull(argv[3],&end,0); if(!*argv[3] || *end || !id) return RETURN_ERROR; }
    lock=Lock((CONST_STRPTR)argv[1],SHARED_LOCK);
    if (!lock) { PrintFault((LONG)IoErr(),(CONST_STRPTR)"AFSPlusSnapshot"); return RETURN_FAIL; }
    port=afsplus_client_lock_port(lock);
    if (!strcmp(argv[2],"CREATE") && argc==3)
    { error=afsplus_client_snapshot_create(port,&id); if(!error){number(id);PutStr("\n");} }
    else if (!strcmp(argv[2],"LIST") && argc==3)
    {
        struct AfsplusArosSnapshotInfo entries[16]; uint64_t low=0,next; uint32_t count,i;
        do {
            error=afsplus_client_snapshot_list(port,low,entries,16,&count,&next);
            for(i=0;!error && i<count;i++){number(entries[i].id);PutStr(" generation ");number(entries[i].generation);PutStr("\n");}
            if(!error && next && next<=low) error=ERROR_BAD_NUMBER;
            low=next;
        } while(!error && low);
    }
    else if (!strcmp(argv[2],"DELETE") && argc==4) error=afsplus_client_snapshot_delete(port,id);
    else if (!strcmp(argv[2],"VIEW") && argc>=4) error=view(port,id,argc==5?argv[4]:NULL);
    else error=ERROR_BAD_NUMBER;
    UnLock(lock);
    if(error){PrintFault(error,(CONST_STRPTR)"AFSPlusSnapshot");return RETURN_FAIL;}
    return RETURN_OK;
}
