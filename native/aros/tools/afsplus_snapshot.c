/* SPDX-License-Identifier: BSD-2-Clause */
/* Copyright (c) 2026 John Knipper */
/* AFSPlusSnapshot DEVICE CREATE|LIST|DELETE id|VIEW id [relative-path]. */
#include <dos/dos.h>
#include <dos/dosextens.h>
#include <dos/dosasl.h>
#include <proto/dos.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#include "../client/afsplus_client.h"

/* Writes text to Output(); a failed write is an error, not a lost line. */
static LONG put(const char *text)
{
    LONG length=(LONG)strlen(text);
    if (Write(Output(),(APTR)text,length)==length) return 0;
    return IoErr()?(LONG)IoErr():ERROR_DISK_FULL;
}
static LONG number(uint64_t n)
{ char text[21]; snprintf(text,sizeof(text),"%llu",(unsigned long long)n); return put(text); }
/* Decimal, or 0x and 1 to 16 hex digits. A leading zero is refused, so 010
 * is neither eight nor ten; no sign, no overflow, not zero. */
static int parse_id(const char *text,uint64_t *id)
{
    uint64_t value=0;
    int hex=text[0]=='0' && (text[1]=='x' || text[1]=='X');
    unsigned base=hex?16:10,digits=0;
    if (hex) text+=2;
    else if (text[0]=='0') return 0;
    for (; *text; text++,digits++)
    {
        unsigned digit;
        if (*text>='0' && *text<='9') digit=(unsigned)(*text-'0');
        else if (hex && *text>='a' && *text<='f') digit=(unsigned)(*text-'a'+10);
        else if (hex && *text>='A' && *text<='F') digit=(unsigned)(*text-'A'+10);
        else return 0;
        if (value>(UINT64_MAX-digit)/base) return 0;
        value=value*base+digit;
    }
    if (!digits) return 0;
    *id=value;
    return value!=0;
}
static LONG interrupted(void)
{ return CheckSignal(SIGBREAKF_CTRL_C)?ERROR_BREAK:0; }
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
            error=interrupted();
            if (!error) error=afsplus_client_snapshot_dir_read(port,dir,buffer,sizeof(buffer),8,&count,&eof);
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
            error=interrupted();
            if (!error) error=afsplus_client_snapshot_read(port,reader,object,offset,buffer,sizeof(buffer),&count);
            if (!error && count && Write(Output(),buffer,(LONG)count)!=(LONG)count)
            { error=(LONG)IoErr(); if(!error)error=ERROR_DISK_FULL; }
            if (!error) offset+=count;
        } while (!error && count);
    }
    else if (!error) error=ERROR_OBJECT_WRONG_TYPE;
    { LONG close_error=afsplus_client_snapshot_close(port,reader); if(!error)error=close_error; }
    return error;
}
int main(int argc,char **argv)
{
    BPTR lock; struct MsgPort *port; LONG error=0;
    uint64_t id=0;
    int create=argc==3 && !strcmp(argv[2],"CREATE"), list=argc==3 && !strcmp(argv[2],"LIST");
    int remove=argc==4 && !strcmp(argv[2],"DELETE"), show=(argc==4 || argc==5) && !strcmp(argv[2],"VIEW");
    if (!(create || list || remove || show) || ((remove || show) && !parse_id(argv[3],&id)))
    {
        PutStr("usage: AFSPlusSnapshot DEVICE CREATE|LIST|DELETE id|VIEW id [path]\n"
               "id is a snapshot number from LIST, in decimal or 0x hexadecimal.\n");
        return RETURN_ERROR;
    }
    lock=Lock((CONST_STRPTR)argv[1],SHARED_LOCK);
    if (!lock) { PrintFault((LONG)IoErr(),(CONST_STRPTR)"AFSPlusSnapshot"); return RETURN_FAIL; }
    port=afsplus_client_lock_port(lock);
    if (create)
    {
        /* The snapshot exists once created: a lost id is still reported. */
        error=afsplus_client_snapshot_create(port,&id);
        if (!error) error=number(id);
        if (!error) error=put("\n");
    }
    else if (list)
    {
        struct AfsplusArosSnapshotInfo entries[16]; uint64_t low=0,next; uint32_t count,i;
        do {
            error=interrupted();
            if (!error) error=afsplus_client_snapshot_list(port,low,entries,16,&count,&next);
            for(i=0;!error && i<count;i++)
            {
                error=number(entries[i].id);
                if (!error) error=put(" generation ");
                if (!error) error=number(entries[i].generation);
                if (!error) error=put("\n");
            }
            if(!error && next && next<=low) error=ERROR_BAD_NUMBER;
            low=next;
        } while(!error && low);
    }
    else if (remove) error=afsplus_client_snapshot_delete(port,id);
    else error=view(port,id,argc==5?argv[4]:NULL);
    UnLock(lock);
    if(error){PrintFault(error,(CONST_STRPTR)"AFSPlusSnapshot");return RETURN_FAIL;}
    return RETURN_OK;
}
