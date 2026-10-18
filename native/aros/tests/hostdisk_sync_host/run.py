#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
import argparse,hashlib,json,subprocess
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('--output',type=Path,required=True);a=p.parse_args();here=Path(__file__).resolve().parent;root=here.parents[3];source=root/'native/aros/afsplus_handler.c';s=source.read_text();code=s[s.index('static int32_t device_sync('):s.index('static uint32_t supports_command(')]
changes={'positive':None,'old-false-barrier-accepted':('device->lib_Revision < 1','device->lib_Revision < 0'),'unsupported-lost':('if (result == IOERR_NOCMD || request->iotd_Req.io_Error == IOERR_NOCMD)','if (0)'),'device-error-lost':('result == 0 && request->iotd_Req.io_Error == 0','result == 0'),'transport-error-lost':('result == 0 && request->iotd_Req.io_Error == 0','request->iotd_Req.io_Error == 0')}
out=a.output.resolve();out.mkdir(parents=True,exist_ok=False);rows=[]
for name,change in changes.items():
 lane=out/name;lane.mkdir();actual=code
 if change:assert actual.count(change[0])==1;actual=actual.replace(*change)
 (lane/'actual.c').write_text(actual);cmd=['clang','-std=c11','-Wall','-Wextra','-Werror','-Wno-tautological-constant-out-of-range-compare','-fsanitize=address,undefined','-g','-I'+str(lane),str(here/'host.c'),'-o',str(lane/'host')];b=subprocess.run(cmd,capture_output=True,text=True);(lane/'compile.log').write_text(b.stdout+b.stderr);r=None
 if not b.returncode:r=subprocess.run([str(lane/'host')],capture_output=True,text=True,timeout=5);(lane/'run.log').write_text(r.stdout+r.stderr)
 rows.append({'name':name,'compile_exit':b.returncode,'run_exit':None if r is None else r.returncode})
record={'scope':'Actual AFS native device_sync body, modeled Exec I/O/versioned devices; no real driver/guest or power-loss claim.','inputs':{str(x.relative_to(root)):hashlib.sha256(x.read_bytes()).hexdigest() for x in [source,here/'host.c',Path(__file__).resolve()]},'checks':rows};(out/'result.json').write_text(json.dumps(record,indent=2)+'\n');print(rows);assert all(r['compile_exit']==0 and(r['run_exit']==0 if r['name']=='positive' else r['run_exit'] not in (0,None)) for r in rows)
