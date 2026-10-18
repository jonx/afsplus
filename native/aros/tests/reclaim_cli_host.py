#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
# Copyright (c) 2026 John Knipper
"""Actual CLI with a bounded maintenance oracle; no guest qualification."""
from pathlib import Path
import os
import re
import subprocess
import tempfile

repo = Path(__file__).resolve().parents[3]
source = (repo / 'native/aros/tools/afsplus_reclaim.c').read_text()
# The CLI must never resolve a cursor pathname or mutate the DOS namespace.
assert not re.search(r'\b(Open|Close|Read|Write|Flush|DeleteFile|Rename|CreateDir|CurrentDir|DoPkt)\s*\(', source)
source = '\n'.join(line for line in source.splitlines() if not line.startswith('#include'))
prefix = r'''
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "afsplus_aros.h"
typedef int32_t LONG;
typedef uintptr_t BPTR;
typedef const char *CONST_STRPTR;
struct MsgPort { int n; };
struct DevProc { struct MsgPort *dvp_Port; };
#define BNULL 0
#define SHARED_LOCK 1
#define SIGBREAKF_CTRL_C 4096
#define RETURN_OK 0
#define RETURN_WARN 5
#define RETURN_ERROR 10
#define RETURN_FAIL 20
#define ERROR_RENAME_ACROSS_DEVICES 215
#define ERROR_BAD_NUMBER 115
#define ERROR_INVALID_LOCK 211
#define ERROR_OBJECT_IN_USE 202
static struct MsgPort afs={1}, ram={2};
static unsigned api_calls, checks, devices, locks;
static const char *mode(void) { const char *m=getenv("MODE");return m?m:"done"; }
static LONG IoErr(void) { return 205; }
static struct DevProc *GetDeviceProc(const char *name,void *old) {
    (void)old;
    if(!strcmp(mode(),"device-fail"))return NULL;
    struct DevProc *d=malloc(sizeof(*d));if(!d)abort();
    d->dvp_Port=!strncmp(name,"AFS:",4)?&afs:&ram;devices++;return d;
}
static void FreeDeviceProc(struct DevProc *d) { if(!devices)abort();devices--;free(d); }
static BPTR Lock(const char *name,int access) {
    (void)access;if(!strcmp(mode(),"lock-fail"))return 0;
    locks++;return !strncmp(name,"AFS:",4)?1:2;
}
static void UnLock(BPTR b) { if(!b||!locks)abort();locks--; }
static struct MsgPort *afsplus_client_lock_port(BPTR b) { return b==1?&afs:&ram; }
static LONG CheckSignal(LONG mask) {
    if(mask!=SIGBREAKF_CTRL_C)abort();checks++;
    return !strcmp(mode(),"interrupt-zero") || (!strcmp(mode(),"interrupt-two")&&checks==3);
}
static unsigned scan_pos;
static intptr_t afsplus_client_restore_reclaim(struct MsgPort *p,BPTR scope,struct AfsplusArosRestoreRecovery *r) {
    if(p!=&afs||scope!=1||r->struct_size!=sizeof(*r))abort();
    if(!r->scan_entries||r->scan_entries>64||!r->work_steps||r->work_steps>8||!r->max_depth||r->max_depth>64)abort();
    if(!api_calls) { if(r->cursor_size)abort(); }
    /* A fresh scan after a stale cursor starts empty; otherwise the cursor
       continues exactly. */
    if(!r->cursor_size) { uint8_t zero[sizeof(r->cursor)]={0};if(memcmp(r->cursor,zero,sizeof(zero)))abort();scan_pos=0; }
    else { uint32_t prior=0;memcpy(&prior,r->cursor,4);if(r->cursor_size!=4||prior!=scan_pos)abort(); }
    api_calls++;
    if(!strcmp(mode(),"stale")&&api_calls==2)return ERROR_INVALID_LOCK;
    if(!strcmp(mode(),"stale-always")&&scan_pos==1)return ERROR_INVALID_LOCK;
    if(!strcmp(mode(),"busy"))return ERROR_OBJECT_IN_USE;
    r->scanned=1;r->removed=1;r->advanced=0;r->flags=0;
    r->blocked=!strcmp(mode(),"blocked")?2:0;
    r->blocked_object=!strcmp(mode(),"blocked")&&api_calls==1?99:0;
    scan_pos++;
    uint32_t next=scan_pos;memcpy(r->cursor,&next,4);r->cursor_size=4;
    if((!strcmp(mode(),"done")||!strcmp(mode(),"blocked")||!strcmp(mode(),"stale"))&&scan_pos==3) {
        r->flags=AFSPLUS_RESTORE_RECOVERY_DONE;r->cursor_size=0;
    }
    return 0;
}
#define main actual_main
'''
suffix = r'''
#undef main
int main(int argc,char **argv) {
    int result=actual_main(argc,argv);
    if(locks||devices)abort();
    fprintf(stderr,"ORACLE calls=%u checks=%u locks=%u devices=%u\n",api_calls,checks,locks,devices);
    return result;
}
'''
with tempfile.TemporaryDirectory(prefix='reclaim-cli-') as directory:
    tmp = Path(directory)
    c, exe = tmp / 'fixture.c', tmp / 'reclaim'
    c.write_text(prefix + source + suffix)
    subprocess.run(['clang', '-std=c11', '-Wall', '-Wextra', '-Werror',
                    '-Wconversion', '-Wsign-conversion', '-fsanitize=address,undefined',
                    '-I' + str(repo / 'api'), str(c), '-o', str(exe)], check=True)
    work = tmp / 'work'; work.mkdir()
    repair = work / 'repair'; repair.mkdir()
    (repair / 'cursor').write_bytes(b'unrelated repair data')
    (work / 'cursor.reclaim-00').symlink_to('repair', target_is_directory=True)
    def inventory():
        return [(str(p.relative_to(work)), 'link:' + os.readlink(p) if p.is_symlink()
                 else 'dir' if p.is_dir() else p.read_bytes()) for p in sorted(work.rglob('*'))]
    original = inventory()
    runs = 0
    def run(mode='done', args=('AFS:', 'AFS:scope'), expected=0, calls=3):
        global runs
        env = os.environ.copy(); env['MODE'] = mode
        result = subprocess.run([str(exe), *args], cwd=work, env=env, capture_output=True, text=True)
        assert result.returncode == expected, (result.returncode, result.stdout, result.stderr)
        assert f'ORACLE calls={calls} ' in result.stderr, result.stderr
        assert inventory() == original
        runs += 1
        return result
    result = run()
    assert 'calls=3 restarts=0 scanned=3 removed=3 advanced=0 blocked=0 blocked-id=0 complete=1\n' in result.stdout
    run(args=('AFS:', 'AFS:scope', '1', '1', '64', '128'))
    run(args=('AFS:', 'AFS:scope', '1', '1', '64', str(2**32-1)))
    result = run('endless', ('AFS:', 'AFS:scope', '1', '1', '64', '2'), 5, 2)
    assert 'complete=0' in result.stdout and 'budget exhausted' in result.stderr
    run('endless', expected=5, calls=4096)
    run()  # Cursor loss/new invocation begins a safe fresh scan.
    result = run('interrupt-zero', expected=5, calls=0)
    assert 'interrupted' in result.stderr and 'complete=0' in result.stdout
    run('interrupt-two', expected=5, calls=2)
    result = run('blocked', expected=5)
    assert 'blocked=2 blocked-id=99 complete=1' in result.stdout  # cumulative, not summed
    # A stale cursor restarts a fresh scan, which completes.
    result = run('stale', expected=0, calls=5)
    assert 'restarts=1 scanned=4' in result.stdout and 'complete=1\n' in result.stdout
    # A volume that keeps changing stops after the bounded restarts.
    result = run('stale-always', expected=20, calls=18)
    assert 'restarts=8' in result.stdout and '8 fresh scans went stale' in result.stderr
    run('busy', expected=20, calls=1)
    run(args=('AFS:', 'RAM:scope'), expected=20, calls=0)
    run('device-fail', expected=20, calls=0)
    run('lock-fail', expected=20, calls=0)
    for values in [('0','1','1'), ('65','1','1'), ('1','9','1'), ('1','1','65'),
                   ('x','1','1'), ('1','1','1','0'), ('1','1','1',str(2**32)),
                   ('1','1','1','9'*80)]:
        # A usage error: RETURN_ERROR with the accepted ranges, nothing touched.
        result = run(args=('AFS:', 'AFS:scope', *values), expected=10, calls=0)
        assert 'scan 1-64' in result.stderr
    result = run('device-fail', expected=20, calls=0)
    assert 'nothing was changed' in result.stderr and 'may have advanced' not in result.stderr
    run(args=('AFS:', 'AFS:scope', 'RAM:old-cursor'), expected=10, calls=0)
    print(f'reclaim CLI PASS: {runs} actual-source sanitizer scenarios; cursor continuity, bounded calls, Ctrl-C, stale, blocked, scope, lifecycle, no filesystem mutation')
