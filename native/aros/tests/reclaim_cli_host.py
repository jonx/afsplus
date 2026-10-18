#!/usr/bin/env python3
"""Production AROS CLI under a POSIX DOS fixture, not a guest qualification."""
from pathlib import Path
import os
import subprocess
import tempfile

repo=Path(__file__).resolve().parents[3]
source=(repo/'native/aros/tools/afsplus_reclaim.c').read_text()
source='\n'.join(line for line in source.splitlines() if not line.startswith('#include'))
prefix=r'''
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>
#include <errno.h>
#include "afsplus_aros.h"
typedef long LONG;
typedef uintptr_t BPTR;
typedef const char *CONST_STRPTR;
struct MsgPort {int n;};
struct DevProc {struct MsgPort *dvp_Port;};
struct {struct {unsigned lib_Version,lib_Revision;} dl_lib;} dos={ {50,84} };
#define DOSBase (&dos)
#define BNULL 0
#define SHARED_LOCK 1
#define MODE_OLDFILE 1
#define MODE_NEWFILE 2
#define RETURN_OK 0
#define RETURN_WARN 5
#define RETURN_ERROR 10
#define RETURN_FAIL 20
#define ERROR_OBJECT_NOT_FOUND 205
#define ERROR_OBJECT_EXISTS 203
#define ERROR_RENAME_ACROSS_DEVICES 215
#define ERROR_BAD_NUMBER 115
#define ERROR_NO_FREE_STORE 103
#define ERROR_INVALID_LOCK 211
#define ERROR_OBJECT_IN_USE 202
static LONG ioerr;
static struct MsgPort afs={1},ram={2};
static const char *mode(void){const char *s=getenv("MODE");return s?s:"step";}
static const char *path(const char *s){return !strncmp(s,"RAM:",4)?s+4:s;}
static LONG IoErr(void){return ioerr;}
static struct DevProc *GetDeviceProc(const char *name,void *old){
    (void)old;struct DevProc *d=malloc(sizeof(*d));d->dvp_Port=!strncmp(name,"AFS:",4)?&afs:&ram;return d;
}
static void FreeDeviceProc(struct DevProc *d){free(d);}
static BPTR Lock(const char *s,int access){(void)access;return !strcmp(s,"RAM:link")?3:(!strncmp(s,"AFS:",4)?1:2);}
static void UnLock(BPTR b){(void)b;}
static struct MsgPort *afsplus_client_lock_port(BPTR b){return b==2?&ram:&afs;}
static BPTR CurrentDir(BPTR b){static BPTR current=2;BPTR old=current;current=b;return old;}
static const char *FilePart(const char *s){const char *p=s;for(;*s;s++)if(*s=='/'||*s==':')p=s+1;return p;}
static const char *PathPart(const char *s){const char *p=s;for(;*s;s++){if(*s=='/')p=s;else if(*s==':')p=s+1;}return p;}
static BPTR Open(const char *s,int mode){FILE *f=fopen(path(s),mode==1?"rb":"wb");if(!f)ioerr=205;return (BPTR)f;}
static LONG Read(BPTR b,void *p,LONG n){return (LONG)fread(p,1,(size_t)n,(FILE*)b);}
static LONG Write(BPTR b,const void *p,LONG n){if(!strcmp(mode(),"write-fail")){ioerr=100;return -1;}return (LONG)fwrite(p,1,(size_t)n,(FILE*)b);}
static LONG Flush(BPTR b){if(!strcmp(mode(),"flush-fail")){ioerr=100;return 0;}return fflush((FILE*)b)==0;}
static LONG Close(BPTR b){return fclose((FILE*)b)==0;}
static BPTR CreateDir(const char *s){if(mkdir(path(s),0700)){ioerr=errno==EEXIST?203:100;return 0;}return 2;}
static LONG DeleteFile(const char *s){if(remove(path(s))==0)return 1;ioerr=errno==ENOENT?205:100;return 0;}
static LONG Rename(const char *from,const char *to){if(!strcmp(mode(),"rename-fail")){ioerr=100;return 0;}return rename(path(from),path(to))==0;}
static LONG afsplus_client_restore_reclaim(struct MsgPort *p,BPTR b,struct AfsplusArosRestoreRecovery *r){
    (void)p;(void)b;
    FILE *calls=fopen("calls","ab");fputc('x',calls);fclose(calls);
    if(!strcmp(mode(),"stale"))return 205;
    if(r->cursor_size && (r->cursor_size!=4||memcmp(r->cursor,"next",4)))abort();
    if(!strcmp(mode(),"done")||!strcmp(mode(),"blocked")){r->flags=1;r->blocked=!strcmp(mode(),"blocked")?2:0;r->removed=1;r->cursor_size=0;}
    else {r->cursor_size=4;memcpy(r->cursor,"next",4);r->scanned=2;}
    return 0;
}
'''
with tempfile.TemporaryDirectory(prefix='reclaim-cli-') as tmp:
    tmp=Path(tmp);c=tmp/'fixture.c';exe=tmp/'reclaim'
    c.write_text(prefix+source)
    subprocess.run(['clang','-Wall','-Wextra','-Werror','-fsanitize=address,undefined',
                    '-I'+str(repo/'api'),str(c),'-o',str(exe)],check=True)
    def run(mode='step',args=('AFS:','AFS:scope','RAM:cursor'),expected=5):
        env=os.environ.copy();env['MODE']=mode
        r=subprocess.run([str(exe),*args],cwd=tmp,env=env,capture_output=True,text=True)
        assert r.returncode==expected,(r.returncode,r.stdout,r.stderr)
        return r
    run();assert (tmp/'cursor').read_bytes()==b'next'
    for failure in ('write-fail','flush-fail','stale'):
        run(failure,expected=20);assert (tmp/'cursor').read_bytes()==b'next'
        assert not list(tmp.glob('cursor.reclaim-*'))
    run('rename-fail',expected=20);assert not (tmp/'cursor').exists()
    run();assert (tmp/'cursor').read_bytes()==b'next'
    run('done',expected=0);assert not (tmp/'cursor').exists()
    r=run('blocked');assert 'blocked=2' in r.stdout and 'Partial:' in r.stderr
    calls=(tmp/'calls').read_bytes()
    run(args=('AFS:','AFS:scope','AFS:cursor'),expected=20)
    run(args=('AFS:','AFS:scope','RAM:link/cursor'),expected=20)
    run(args=('AFS:','AFS:scope','RAM:cursor','65','1','64'),expected=20)
    assert (tmp/'calls').read_bytes()==calls
    (tmp/'cursor.reclaim-00').mkdir();(tmp/'cursor.reclaim-00/unknown').write_bytes(b'keep')
    run();assert (tmp/'cursor.reclaim-00/unknown').read_bytes()==b'keep'
    print('reclaim CLI PASS: resume, done, blocked, external cursor, budgets, exclusive save, write/flush/stale preservation, lost cursor rename recovery')
