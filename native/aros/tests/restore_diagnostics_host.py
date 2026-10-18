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
#define Printf printf
#define CONST_STRPTR const char *
int main(int argc,char **argv) {
 if(argc!=4)return 7;
 int error=atoi(argv[1]),attempted=1,published=atoi(argv[2]);
 uint64_t reader=0x12,object=0x34;
 const char *phase=published?"snapshot-close":"restore";
 char *paths[]={"unused","unused","unused","unused","AFSPROBE:Restore Dir","new file"};
 argv=paths;
'''
with tempfile.TemporaryDirectory(prefix='afs-restore-diag-') as td:
 p=Path(td);c=p/'test.c';b=p/'test'
 def compile(text):
  c.write_text(prefix+text+'\nreturn 0;\n}\n')
  subprocess.run(['clang','-std=c11','-Wall','-Wextra','-Werror','-g','-fsanitize=address,undefined',str(c),'-o',str(b)],check=True)
 def check():
  uncertain=subprocess.run([str(b),'225','0','fixture'],capture_output=True,text=True,check=True)
  assert not uncertain.stdout
  assert 'restore: DOS error225' in uncertain.stderr.replace('error 225','error225')
  assert 'Publication or cleanup is uncertain.' in uncertain.stderr
  assert '.afsplus-restore-0000000000000012-0000000000000034-*' in uncertain.stderr
  assert 'destination new file' in uncertain.stderr and 'AFSPROBE:Restore Dir' in uncertain.stderr
  closed=subprocess.run([str(b),'225','1','fixture'],capture_output=True,text=True,check=True)
  assert not closed.stdout and 'was published' in closed.stderr
  assert 'Publication or cleanup is uncertain.' not in closed.stderr
  denied=subprocess.run([str(b),'214','0','fixture'],capture_output=True,text=True,check=True)
  assert not denied.stdout and 'DOS error214' in denied.stderr.replace('error 214','error214')
  assert 'staged files' not in denied.stderr
 compile(code);check();print('PASS uncertain restore reports stderr destination/stage pattern; published close failure and denied admission distinguished')
 compile(code.replace('fprintf(stderr, ','printf('))
 try:check()
 except AssertionError:print('PASS stdout diagnostic mutation rejected')
 else:raise AssertionError('stdout diagnostic control unexpectedly accepted')
