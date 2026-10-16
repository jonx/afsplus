#!/usr/bin/env python3
# Copyright (C) 2026 John KNIPPER <code@jkn.me>
# Distributed under the AROS Public License 1.1.
"""Run the actual handler clock callback at a deterministic timer boundary.

This extracts the production function, not a rewrite. It checks epoch/locale
conversion, real microseconds, early coarse fallback and a rejected coarse
clock mutant. It does not qualify a guest timer or filesystem.
"""
from pathlib import Path
import argparse,hashlib,json,re,subprocess
p=argparse.ArgumentParser(description=__doc__);p.add_argument('--output',type=Path,required=True);a=p.parse_args()
source=Path(__file__).resolve().parents[1]/'afsplus_handler.c';out=a.output.resolve();out.mkdir(parents=True,exist_ok=False)
s=source.read_text();m=re.search(r'static int32_t packet_now\(void \*context, int64_t \*unix_seconds,\s*uint32_t \*nanoseconds\)\s*\{',s);assert m
end=s.index('\n}\n',m.end())+2;body=s[m.start():end]
header=r'''
#include <assert.h>
#include <stdint.h>
#include <stddef.h>
#define ERROR_BAD_NUMBER 115
#define AFSPLUS_SECONDS_PER_MINUTE INT64_C(60)
#define AFSPLUS_SECONDS_PER_DAY INT64_C(86400)
#define AFSPLUS_TICKS_PER_SECOND UINT32_C(50)
#define AFSPLUS_UNIX_TO_AMIGA_EPOCH INT64_C(252460800)
struct Device { int marker; };
struct DosLibrary { int marker; };
struct Locale { int loc_GMTOffset; };
struct timeval { uint32_t tv_secs, tv_micro; };
struct DateStamp { int32_t ds_Days, ds_Minute, ds_Tick; };
struct timerequest { struct { struct Device *io_Device; } tr_node; };
struct AfsplusArosHandler { struct DosLibrary *DOSBase; struct Locale *locale; int timer_open; struct timerequest *timer_request; };
static struct Device device;
static struct timeval clock_value;
static struct DateStamp coarse_value;
static unsigned precise_calls,coarse_calls;
static void precise(struct Device *base,struct timeval *out) { assert(base==&device); ++precise_calls; *out=clock_value; }
static void coarse(struct DosLibrary *base,struct DateStamp *out) { assert(base); ++coarse_calls; *out=coarse_value; }
#define GetSysTime(p) precise(TimerBase,p)
#define DateStamp(p) coarse(DOSBase,p)
'''
checks=r'''
int main(void) {
 struct DosLibrary dos={0}; struct Locale locale={60}; struct timerequest timer={{&device}};
 struct AfsplusArosHandler handler={&dos,&locale,1,&timer};
 int64_t sec=0;uint32_t ns=0;
 clock_value=(struct timeval){1000,123456};
 assert(packet_now(&handler,&sec,&ns)==0 && sec==252465400 && ns==123456000);
 locale.loc_GMTOffset=-60;
 assert(packet_now(&handler,&sec,&ns)==0 && sec==252458200 && ns==123456000);
 handler.locale=NULL;clock_value=(struct timeval){0,999999};
 assert(packet_now(&handler,&sec,&ns)==0 && sec==252460800 && ns==999999000);
 clock_value.tv_micro=1000000;assert(packet_now(&handler,&sec,&ns)==ERROR_BAD_NUMBER);
 assert(packet_now(&handler,NULL,&ns)==ERROR_BAD_NUMBER);
 assert(packet_now(&handler,&sec,NULL)==ERROR_BAD_NUMBER);
 assert(precise_calls==4 && coarse_calls==0);
 handler.timer_open=0;coarse_value=(struct DateStamp){1,2,3};
 assert(packet_now(&handler,&sec,&ns)==0 && sec==252547320 && ns==60000000);
 coarse_value.ds_Days=-1;assert(packet_now(&handler,&sec,&ns)==ERROR_BAD_NUMBER);
 assert(precise_calls==4 && coarse_calls==2);
 return 0;
}
'''
records=[]
for name,code,negative in [('production',body,False),('control-coarse',body.replace('now.tv_micro * UINT32_C(1000)','(now.tv_micro / UINT32_C(20000)) * UINT32_C(20000000)'),True)]:
 c=out/(name+'.c');c.write_text(header+code+checks);exe=out/name
 cmd=['clang','-std=c11','-Wall','-Wextra','-Werror','-fsanitize=address,undefined',str(c),'-o',str(exe)]
 with(out/(name+'-build.log')).open('w') as log:built=subprocess.run(cmd,stdout=log,stderr=subprocess.STDOUT)
 ran=None
 if built.returncode==0:
  with(out/(name+'.log')).open('w') as log:ran=subprocess.run([str(exe)],stdout=log,stderr=subprocess.STDOUT).returncode
 records.append({'name':name,'command':cmd,'compile_exit':built.returncode,'run_exit':ran,'pass':built.returncode==0 and ran is not None and ((ran!=0) if negative else ran==0)})
result={'result':'PASS' if all(x['pass'] for x in records) else 'FAIL','source_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'records':records,'scope':'actual callback at mocked timer boundary, no guest timer proof'}
(out/'result.json').write_text(json.dumps(result,indent=2)+'\n');print(result['result']);raise SystemExit(result['result']!='PASS')
