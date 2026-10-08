#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
# Copyright (c) 2026 John Knipper
"""Exercise two separately compiled claim modules with eight host threads.

--headers is the generated include directory from dev-packet-matrix.sh.
Exec's resource bootstrap and process lifetime primitive are mocked here.
"""
import argparse
import json
from pathlib import Path
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--aros-source', type=Path, required=True)
parser.add_argument('--headers', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--sdk-include', type=Path, help='generated SDK headers for resource fixture')
parser.add_argument('--resource', action='store_true', help='exercise actual resource publication glue')
parser.add_argument('--sanitize', choices=['address,undefined', 'thread'], default='address,undefined')
args = parser.parse_args()
repo = Path(__file__).resolve().parents[1]
out = args.output.resolve()
out.mkdir(parents=True, exist_ok=False)
stdc = args.aros_source / 'compiler/crt/stdc/include'
flags = ['-std=c11', '-O1', '-g', '-Wall', '-Wextra', '-Werror', '-fno-omit-frame-pointer',
         '-fsanitize=' + args.sanitize]
includes = ['-nostdlibinc', '-D__WORDSIZE=64', '-DAROS_FAST_BPTR=1', '-DAROS_FAST_BSTR=1']
for directory in [args.headers, stdc, stdc / 'aros/stdc', args.aros_source / 'compiler/include',
                  repo / 'native/aros/tests/dev-proto', repo / 'native/aros']:
    includes += ['-I', str(directory.resolve())]
records = []

def run(command):
    result = subprocess.run(command, cwd=repo, text=True, capture_output=True, timeout=120)
    records.append(dict(command=command, returncode=result.returncode,
                        stdout=result.stdout, stderr=result.stderr))
    (out / 'result.json').write_text(json.dumps(records, indent=2) + '\n')
    if result.returncode:
        print(result.stdout + result.stderr)
        raise SystemExit(result.returncode)
    print(result.stdout, end='')

if args.resource:
    includes = ['-D__AROSEXEC_SMP__', '-I', str(repo / 'native/aros/tests/claim-proto'), *includes]
if args.sdk_include:
    includes += ['-I', str(args.sdk_include.resolve())]
objects = []
for prefix in ['copy_a', 'copy_b']:
    obj = out / (prefix + '.o')
    defines = ['-Dafsplus_claim_' + name + '=' + prefix + '_' + name
               for name in (['store', 'lock', 'unlock', 'task_id', 'deliver', 'yield'] if args.resource
                            else ['name', 'take', 'release', 'enroll', 'forward'])]
    run(['clang', *flags, *includes, *defines, '-c', 'native/aros/afsplus_claim_exec.c' if args.resource else 'native/aros/afsplus_claim.c', '-o', str(obj)])
    objects.append(str(obj))
for source, extra in [('claim_resource_stub' if args.resource else 'claim_concurrent_stub', includes),
                      ('claim_threads', ['-pthread'])]:
    obj = out / (source + '.o')
    run(['clang', *flags, *extra, '-c', 'native/aros/tests/' + source + '.c', '-o', str(obj)])
    objects.append(str(obj))
run(['clang', *flags, '-pthread', *objects, '-o', str(out / 'test')])
run([str(out / 'test')])
