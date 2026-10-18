#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
# Copyright (c) 2026 John Knipper
"""Execute the actual task-local table and destructor code with modeled Exec."""
from pathlib import Path
import subprocess
import tempfile
root=Path(__file__).resolve().parents[3]
s=(root/'native/aros/afsplus_bootthread.c').read_text()
code=s[s.index('#define KEY_SLOTS'):s.index('/*\n * emutls takes its mutex')]
prefix=r'''
#include <assert.h>
#include <stdint.h>
#include <stdlib.h>
#include <stdio.h>
#define MEMF_PUBLIC 1
#define MEMF_CLEAR 2
#define AFSPLUS_EINVAL 22
#define AFSPLUS_EPERM 1
#define bug(...) ((void)0)
struct Task { int id; };
static struct Task tasks[64],*current;
static unsigned allocations, depth, fail_alloc;
static struct Task *FindTask(void *p) { (void)p; return current; }
static void Forbid(void) { depth++; }
static void Permit(void) { assert(depth); depth--; }
static void *AllocVec(size_t n, int flags) { (void)flags; assert(!depth); if(fail_alloc)return NULL; void *p=calloc(1,n); assert(p); allocations++; return p; }
static void FreeVec(void *p) { assert(p && allocations && !depth); allocations--; free(p); }
'''
main=r'''
static unsigned destructor_calls, key;
static void destroy(void *value) {
 assert(value==(void *)(uintptr_t)(current->id+1)); assert(!depth);
 destructor_calls++;
 /* A destructor that rearms itself must run only four times. */
 assert(pthread_setspecific(key,value)==0);
}
int main(void) {
 assert(pthread_key_create(&key,destroy)==0);
 for(unsigned wave=0;wave<4;wave++) {
  for(unsigned i=0;i<64;i++) {
   current=&tasks[i]; current->id=i;
   assert(pthread_setspecific(key,(void *)(uintptr_t)(i+1))==0);
  }
  assert(allocations==64);
  for(unsigned i=0;i<64;i++) {
   current=&tasks[i]; assert(pthread_getspecific(key)==(void *)(uintptr_t)(i+1));
   afsplus_aros_thread_cleanup(); afsplus_aros_thread_cleanup();
  }
  assert(allocations==0 && key_tables==NULL);
 }
 assert(destructor_calls==1024);
 current=&tasks[0]; fail_alloc=1;
 assert(pthread_setspecific(key,(void *)1)==AFSPLUS_EINVAL);
 assert(pthread_getspecific(key)==NULL); fail_alloc=0;
 assert(pthread_setspecific(key,(void *)1)==0);
 assert(pthread_key_delete(key)==0);
 afsplus_aros_thread_cleanup(); assert(allocations==0 && destructor_calls==1024);
 puts("PASS 4 waves of64 isolated task tables,1024 destructor calls, allocation refusal,key deletion and repeated cleanup");
}
'''
with tempfile.TemporaryDirectory(prefix='afs-tls-host-') as td:
 p=Path(td); c=p/'test.c'; c.write_text(prefix+code+main); binary=p/'test'
 subprocess.run(['clang','-std=c11','-Wall','-Wextra','-Werror','-fsanitize=address,undefined','-g',str(c),'-o',str(binary)],check=True)
 subprocess.run([str(binary)],check=True)
 # A negative control imposing the historical fourth-owner ceiling must fail.
 c.write_text(prefix+code.replace('    table = AllocVec(sizeof(*table), MEMF_PUBLIC | MEMF_CLEAR);','    if (allocations >= 4) return NULL;\n    table = AllocVec(sizeof(*table), MEMF_PUBLIC | MEMF_CLEAR);')+main)
 subprocess.run(['clang','-std=c11','-g',str(c),'-o',str(binary)],check=True)
 result=subprocess.run([str(binary)],capture_output=True,text=True)
 assert result.returncode!=0
 print('PASS historical fourth-owner ceiling rejected by same64-task workload')
