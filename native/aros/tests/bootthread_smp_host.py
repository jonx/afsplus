#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
# Copyright (c) 2026 John Knipper
"""Run shared resident TLS/once/lazy mutex bodies under64 real host threads."""
from pathlib import Path
import subprocess
import tempfile
s=(Path(__file__).resolve().parents[1]/'afsplus_bootthread.c').read_text()
code=s[s.index('#define KEY_SLOTS'):]
for name in ['key_create','key_delete','setspecific','getspecific','mutex_lock','mutex_unlock','once']:
 code=code.replace('pthread_'+name,'afs_'+name)
prefix=r'''
#define _DARWIN_C_SOURCE
#include <assert.h>
#include <stdint.h>
#include <stdlib.h>
#include <stdio.h>
#include <pthread.h>
#include <stdatomic.h>
#include <unistd.h>
#include <sched.h>
#define MEMF_PUBLIC 1
#define MEMF_CLEAR 2
#define AFSPLUS_EINVAL 22
#define AFSPLUS_EPERM 1
#define MUTEX_MAGIC UINT32_C(0x4166534D)
#define bug(...) ((void)0)
struct SignalSemaphore { pthread_mutex_t m; };
struct boot_mutex { uint32_t magic,pad; struct SignalSemaphore semaphore; };
struct Task { int id; };
static struct Task tasks[64];
static _Thread_local struct Task *current;
static _Thread_local unsigned depth;
static atomic_uint allocations, semaphore_inits;
static struct Task *FindTask(void *p) { (void)p; return current; }
static void Forbid(void) { depth++; }
static void Permit(void) { assert(depth); depth--; }
static void *AllocVec(size_t n,int flags) { (void)flags; assert(!depth); void*p=calloc(1,n); assert(p); atomic_fetch_add(&allocations,1); return p; }
static void FreeVec(void*p) { assert(p && !depth); atomic_fetch_sub(&allocations,1); free(p); }
static void InitSemaphore(struct SignalSemaphore *s) {
 pthread_mutexattr_t a; assert(!pthread_mutexattr_init(&a));
 assert(!pthread_mutexattr_settype(&a,PTHREAD_MUTEX_RECURSIVE));
 assert(!pthread_mutex_init(&s->m,&a)); pthread_mutexattr_destroy(&a);
 atomic_fetch_add(&semaphore_inits,1);
}
static void ObtainSemaphore(struct SignalSemaphore*s) { assert(!depth); assert(!pthread_mutex_lock(&s->m)); }
static void ReleaseSemaphore(struct SignalSemaphore*s) { assert(!pthread_mutex_unlock(&s->m)); }
'''
main=r'''
static struct boot_once once={0,-1,0};
static atomic_uint start,ready,init_calls,destructor_calls;
static unsigned key;
static void destroy(void*p) { assert(p==(void *)(uintptr_t)(current->id+1)); atomic_fetch_add(&destructor_calls,1); }
static void initialize(void) {
 atomic_fetch_add(&init_calls,1); usleep(10000);
 assert(!afs_key_create(&key,destroy)); atomic_store(&ready,1);
}
static void*worker(void*arg) {
 current=arg; while(!atomic_load(&start)) sched_yield();
 assert(!afs_once(&once,initialize)); assert(atomic_load(&ready)==1);
 for(unsigned i=0;i<100;i++) {
  void*p=(void *)(uintptr_t)(current->id+1);
  assert(!afs_setspecific(key,p)); assert(afs_getspecific(key)==p);
  afsplus_aros_thread_cleanup();
 }
 return NULL;
}
int main(void) {
 pthread_t workers[64];
 for(unsigned i=0;i<64;i++) { tasks[i].id=i; assert(!pthread_create(&workers[i],NULL,worker,&tasks[i])); }
 atomic_store(&start,1);
 for(unsigned i=0;i<64;i++) assert(!pthread_join(workers[i],NULL));
 assert(init_calls==1 && semaphore_inits==1 && destructor_calls==6400 && allocations==0 && key_tables==NULL);
 puts("PASS64 concurrent threads,6400 task table lifetimes/destructors,one lazy semaphore and once initializer");
}
'''
with tempfile.TemporaryDirectory(prefix='afs-tls-smp-') as td:
 p=Path(td);c=p/'test.c';c.write_text(prefix+code+main);b=p/'test'
 subprocess.run(['clang','-std=c11','-O1','-g','-pthread','-Wall','-Wextra','-Werror','-fsanitize=address,undefined',str(c),'-o',str(b)],check=True)
 subprocess.run([str(b)],check=True)
 # Publish the once done flag before initialization: peers must reject it.
 bad=code.replace('        routine();','        __atomic_store_n(&once->done, 1, __ATOMIC_RELEASE);\n        routine();')
 c.write_text(prefix+bad+main)
 subprocess.run(['clang','-std=c11','-O1','-pthread',str(c),'-o',str(b)],check=True)
 r=subprocess.run([str(b)],capture_output=True)
 assert r.returncode!=0
 print('PASS pre-initialization once publication rejected by concurrent workload')
