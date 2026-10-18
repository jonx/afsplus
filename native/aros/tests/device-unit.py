#!/usr/bin/env python3
# Copyright (C) 2026 John KNIPPER <code@jkn.me>
# Distributed under the AROS Public License 1.1.
"""Compile the actual native open_device function against an Exec boundary."""
from pathlib import Path
import argparse, hashlib, json, subprocess
p = argparse.ArgumentParser(); p.add_argument('--output', type=Path, required=True); a = p.parse_args()
source = Path(__file__).resolve().parents[1] / 'afsplus_handler.c'
s = source.read_text(); start = s.index('static int32_t open_device(')
body = s[start:s.index('\nstatic int32_t prepare_filesystem(', start)]
out = a.output.resolve(); out.mkdir(parents=True, exist_ok=False)
prefix = r'''
#include <assert.h>
#include <stdint.h>
#include <stddef.h>
#include <string.h>
typedef uintptr_t IPTR;
typedef uint32_t ULONG;
#define UINT32_MAX_TEST UINT32_MAX
#define ERROR_BAD_NUMBER 115
#define ERROR_NO_FREE_STORE 103
#define ERROR_DEVICE_NOT_MOUNTED 218
#define AROS_BSTR_ADDR(x) (x)
struct ExecBase { int unused; };
struct IORequest { int unused; };
struct IOExtTD { struct IORequest request; };
struct FileSysStartupMsg { IPTR fssm_Unit; const char *fssm_Device; ULONG fssm_Flags; };
struct AfsplusArosHandler { struct ExecBase *SysBase; struct FileSysStartupMsg *startup;
 void *device_port; struct IOExtTD *device_request; int device_open; };
static IPTR observed_unit;
static int calls, open_error, media_error, fail_port, fail_request;
static const char *stage;
static struct IOExtTD request;
static void set_startup_stage(struct AfsplusArosHandler *h, const char *s) { (void)h; stage=s; }
static void *CreateMsgPort(void) { return fail_port ? NULL : &calls; }
static void *CreateIORequest(void *p,size_t n) { assert(p && n==sizeof(request)); return fail_request ? NULL : &request; }
static int OpenDevice(const char *name,IPTR unit,struct IORequest *r,ULONG flags) {
 assert(!strcmp(name,"hostdisk.device") && r==(void *)&request && flags==7);
 observed_unit=unit; calls++; return open_error;
}
static int32_t require_media(struct AfsplusArosHandler *h) { assert(h->device_open); return media_error; }
'''
# SysBase is consumed by real AROS Exec macros; boundary functions do not use it.
main = r'''
int main(void) {
 struct ExecBase exec;
 struct FileSysStartupMsg startup={0,"hostdisk.device",7};
 struct AfsplusArosHandler h={.SysBase=&exec,.startup=&startup};
 IPTR units[]={0,1023,UINT32_MAX,UINT64_C(0x100001234),UINTPTR_MAX};
 for(size_t i=0;i<sizeof(units)/sizeof(units[0]);i++) {
  startup.fssm_Unit=units[i]; h.device_open=0; int before=calls;
  assert(open_device(&h)==0); assert(calls==before+1);
  assert(observed_unit==units[i] && h.device_open && !strcmp(stage,"media-state"));
 }
 h.device_open=0; open_error=1; assert(open_device(&h)==ERROR_DEVICE_NOT_MOUNTED); assert(!h.device_open); open_error=0;
 media_error=42; assert(open_device(&h)==42); media_error=0;
 fail_port=1; assert(open_device(&h)==ERROR_NO_FREE_STORE); fail_port=0;
 fail_request=1; assert(open_device(&h)==ERROR_NO_FREE_STORE);
 return 0;
}
'''
records=[]
for case in ['production','control-clamp','control-truncate']:
 actual=body
 if case=='control-clamp': actual=actual.replace('    set_startup_stage(handler, "device-port");','    if (startup->fssm_Unit > UINT32_MAX) return ERROR_BAD_NUMBER;\n    set_startup_stage(handler, "device-port");')
 if case=='control-truncate': actual=actual.replace('(IPTR)startup->fssm_Unit','(ULONG)startup->fssm_Unit')
 path=out/(case+'.c');path.write_text(prefix+actual+main);binary=out/case
 cmd=['clang','-std=c11','-Wall','-Wextra','-Werror','-Wno-unused-variable','-fsanitize=address,undefined','-g',str(path),'-o',str(binary)]
 with (out/(case+'-build.log')).open('w') as log:q=subprocess.run(cmd,stdout=log,stderr=subprocess.STDOUT)
 records.append({'name':case+'-build','command':cmd,'exit':q.returncode,'pass':q.returncode==0})
 if not q.returncode:
  with (out/(case+'.log')).open('w') as log:q=subprocess.run([str(binary)],stdout=log,stderr=subprocess.STDOUT)
  records.append({'name':case,'exit':q.returncode,'pass':q.returncode==0 if case=='production' else q.returncode!=0})
result={'result':'PASS' if all(r['pass'] for r in records) else 'FAIL','records':records,'inputs':{str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in [source,Path(__file__)]},'scope':'Actual native open_device function, modeled Exec device boundary, 64-bit unit identity and failure paths; no hosted filesystem durability claim.'}
(out/'result.json').write_text(json.dumps(result,indent=2)+'\n');print(result['result']);raise SystemExit(result['result']!='PASS')
