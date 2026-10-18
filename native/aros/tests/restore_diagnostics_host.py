#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
# Copyright (c) 2026 John Knipper
"""Execute the actual command's diagnostic block; no DOS/VM claim."""
from pathlib import Path
import subprocess,tempfile
s=(Path(__file__).resolve().parents[1]/'tools/afsplus_restore.c').read_text()
a=s.index('    if (error != 0)\n    {',s.index('done:'))
b=s.index('    return error == 0 ?',a)
code=s[a:b]
prefix=r'''
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#define ERROR_NOT_A_DOS_DISK 225
#define ERROR_UNKNOWN 999
#define ERROR_DISK_FULL 221
#define ERROR_READ_PROTECTED 224
#define ERROR_OBJECT_EXISTS 203
#define AFSPLUS_ERROR_PUBLICATION_UNCERTAIN 50001
#define Printf printf
#define CONST_STRPTR const char *
int main(int argc,char **argv) {
 if(argc!=4)return 7;
 int error=atoi(argv[1]),attempted=1,published=atoi(argv[2]);
 const char *phase=published?"snapshot-close":"restore";
 char *paths[]={"unused","unused","unused","unused","AFSPROBE:Restore Dir","new file"};
 argv=paths;
'''
with tempfile.TemporaryDirectory(prefix='afs-restore-diag-') as td:
 p=Path(td);c=p/'test.c';b=p/'test'
 def compile(text):
  c.write_text(prefix+text+'\nreturn 0;\n}\n')
  subprocess.run(['clang','-std=c11','-Wall','-Wextra','-Werror','-g','-fsanitize=address,undefined',str(c),'-o',str(b)],check=True)
 def run(*args):
  r=subprocess.run([str(b),*args,'fixture'],capture_output=True,text=True,check=True)
  assert not r.stdout
  return r.stderr.replace('DOS error ','DOS error')
 def check():
  uncertain=run('50001','0')
  assert 'restore: DOS error50001' in uncertain
  assert 'Remount and inspect destination new file in AFSPROBE:Restore Dir' in uncertain
  assert 'AFSPlusReclaim' not in uncertain
  stage=run('225','0')
  assert 'Run AFSPlusReclaim on destination directory AFSPROBE:Restore Dir' in stage
  assert 'Remount' not in stage
  closed=run('225','1')
  assert 'snapshot-close: DOS error225' in closed and 'was published' in closed
  assert 'AFSPlusReclaim' not in closed and 'Remount' not in closed
  denied=run('224','0')
  assert 'DOS error224' in denied
  assert 'AFSPlusReclaim' not in denied and 'Remount' not in denied and 'published' not in denied
 compile(code);check();print('PASS uncertain publication, retained stage, published close failure and denied admission each get their own stderr advice')
 compile(code.replace('fprintf(stderr, ','printf('))
 try:check()
 except AssertionError:print('PASS stdout diagnostic mutation rejected')
 else:raise AssertionError('stdout diagnostic control unexpectedly accepted')
