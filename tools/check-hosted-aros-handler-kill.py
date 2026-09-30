#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""Kill an isolated hosted guest under an active write workload, then remount.

The instance directory contains AROS/, an executable control wrapper with
isolated BOOTD/FIFO/PIDF/LOG/JOB settings, startup, guest.pid and guest.log.
The wrapper maps MacRW: to instance/results. Never pass the shared instance.
Build AFSPlusKillProbe from native/aros/tests/kill_probe.c into that tree's C/.
Install the candidate handler as L:afsplus-handler. The test owns only Unit28.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import time


def require(condition, reason):
    if not condition:
        raise RuntimeError(reason)


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--instance', type=Path, required=True)
    parser.add_argument('--mkafsplus', type=Path, required=True)
    parser.add_argument('--checker', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--timeout', type=float, default=90)
    args = parser.parse_args()
    instance, output = args.instance.resolve(), args.output.resolve()
    require(not output.exists(), 'refuse to overwrite retained evidence')
    require(instance != Path.home() / 'aros-build', 'an isolated instance is required')
    control = instance / 'control'
    text = control.read_text()
    for key, value in [('AROS_CTL_BOOTD', instance / 'AROS/boot/darwin'),
                       ('AROS_CTL_PIDF', instance / 'guest.pid'),
                       ('AROS_CTL_FIFO', instance / 'control.fifo'),
                       ('AROS_CTL_JOB_PLIST', instance / 'guest.plist'),
                       ('AROS_CTL_LOG', instance / 'guest.log'),
                       ('AROS_CTL_HOST_FOLDER', instance / 'results'),
                       ('AROS_CTL_STARTUP_FILE', instance / 'startup')]:
        assignment = re.search(rf'^export {key}=(.+)$', text, re.M)
        require(assignment and Path(assignment[1]).resolve() == value,
                f'control wrapper must isolate {key}')
    label = re.search(r'^export AROS_CTL_JOB_LABEL=([A-Za-z0-9_.-]+)$', text, re.M)
    require(label and label[1] == 'org.aros.' + instance.name, 'launchd label must include isolated instance name')
    tree = instance / 'AROS'
    require((tree / 'C/AFSPlusKillProbe').is_file(), 'missing target kill probe')
    require((tree / 'L/afsplus-handler').is_file(), 'missing candidate handler')
    image = tree / 'DiskImages/Unit28'
    require(not image.exists(), 'Unit28 already exists; never replace another image')
    output.mkdir(parents=True)
    results = instance / 'results'
    results.mkdir(exist_ok=True)
    report = {'result': 'FAIL', 'model': 'hosted-process-SIGKILL-host-cache-survives',
              'handler_sha256': sha(tree / 'L/afsplus-handler'), 'cases': []}

    def ctl(verb):
        return subprocess.run([str(control), verb], capture_output=True, text=True,
                              timeout=args.timeout)

    def owned_pids():
        listing = subprocess.check_output(['ps', '-axo', 'pid=,command='], text=True)
        return [int(line.split(None, 1)[0]) for line in listing.splitlines()
                if any(prefix in line for prefix in [str(tree / 'boot/darwin'),
                                                  str(tree / 'boot/darwin').replace('/private/tmp/', '/tmp/')])
                and ('Macaros' in line or 'AROSBootstrap' in line)]

    def kill_owned():
        pid = int((instance / 'guest.pid').read_text().strip())
        require(pid in owned_pids(),
                f'pid {pid} does not belong to this copied boot tree')
        os.kill(pid, signal.SIGKILL)
        deadline = time.monotonic() + 10
        while pid in owned_pids() and time.monotonic() < deadline:
            time.sleep(0.02)
        require(pid not in owned_pids(), 'SIGKILL did not terminate owned guest')
        return pid

    def phase(name, commands, marker, cut_ack=None):
        log = results / f'{name}.out'
        require(not log.exists(), f'refuse previous guest result {log}')
        script = ('FailAt 1000\nAssign FDSK: SYS:DiskImages\n'
                  'C:Mount SYS:Devs/DOSDrivers/AFSCUT\n')
        script += ''.join(f'C:AFSPlusKillProbe {c} >>MacRW:{name}.out\n' for c in commands)
        (instance / 'startup').write_text(script)
        started = ctl('run')
        (output / f'{name}-launch.txt').write_text(started.stdout + started.stderr)
        require(started.returncode == 0, f'{name}: guest launch failed')
        deadline = time.monotonic() + args.timeout
        try:
            while time.monotonic() < deadline:
                content = log.read_text(errors='replace') if log.exists() else ''
                require('[AFSPLUS-KILL] FAIL' not in content, f'{name}: guest failure')
                require('EXHAUSTED' not in content, f'{name}: failed to kill active workload')
                if cut_ack is not None:
                    acks = [int(n) for n in re.findall(r'\[AFSPLUS-KILL\] ACK (\d+)', content)]
                    writing = [int(n) for n in re.findall(r'\[AFSPLUS-KILL\] WRITING (\d+)', content)]
                    if acks and max(acks) >= cut_ack and writing and max(writing) > max(acks):
                        pid = kill_owned()
                        final = log.read_text(errors='replace')
                        require('[AFSPLUS-KILL] FAIL' not in final and 'EXHAUSTED' not in final,
                                'workload failed or finished before observed termination')
                        final_acks = [int(n) for n in re.findall(r'\[AFSPLUS-KILL\] ACK (\d+)\n', final)]
                        final_writes = [int(n) for n in re.findall(r'\[AFSPLUS-KILL\] WRITING (\d+)\n', final)]
                        require(final_acks and final_writes, 'missing final workload bounds')
                        require(max(final_acks) <= max(final_writes), 'invalid workload bounds')
                        return {'signal': 9, 'pid': pid, 'acknowledged': max(final_acks),
                                'last_attempted': max(final_writes),
                                'writing_at_observation': max(writing),
                                'termination_observed': True}
                elif marker in content:
                    return content
                time.sleep(0.02)
            raise RuntimeError(f'{name}: timed out waiting for {marker}')
        finally:
            # Preserve evidence before the private wrapper cleans up launchd.
            if log.exists():
                shutil.copy2(log, output / log.name)
            guest_log = instance / 'guest.log'
            if guest_log.exists():
                shutil.copy2(guest_log, output / f'{name}-guest.log')
            stopped = ctl('stop')
            (output / f'{name}-stop.txt').write_text(stopped.stdout + stopped.stderr)
            require(stopped.returncode == 0 and not owned_pids(),
                    f'{name}: private guest still running after stop')
            scanner = Path(__file__).with_name('check-aros-serial-log.sh')
            for diagnostic in [output / log.name, output / f'{name}-guest.log']:
                subprocess.run([str(scanner), str(diagnostic)], check=True)

    try:
        require(not owned_pids(), 'private instance must be stopped')
        image.parent.mkdir(exist_ok=True)
        subprocess.run([str(args.mkafsplus.resolve()), str(image), '--size-mib', '32',
                        '--case-insensitive', '--label', 'Cut'], check=True)
        mountlist = ('FileSystem=L:afsplus-handler\n Device=fdsk.device\n'
                     ' Unit=28\n Surfaces=1\n BlocksPerTrack=1\n LowCyl=0\n'
                     ' HighCyl=8191\n BlockSize=4096\n Reserved=0\n Buffers=64\n'
                     ' BufMemType=1\n Mask=0\n StackSize=524288\n Priority=5\n'
                     ' GlobVec=-1\n DosType=0x4146532b\n Control="COMMIT=SYNC"\n Activate=1\n')
        (tree / 'Devs/DOSDrivers/AFSCUT').write_text(mountlist)
        phase('prepare', ['prepare'], '[AFSPLUS-KILL] PREPARED')
        baseline = output / 'baseline.img'
        shutil.copy2(image, baseline)
        for threshold in [5, 13, 29]:
            name = f'cut-{threshold}'
            shutil.copy2(baseline, image)
            case = phase(name, ['stress'], 'active writing', cut_ack=threshold)
            shutil.copy2(image, output / f'{name}-before-recovery.img')
            recovered = phase(f'{name}-verify', [f'verify {case["acknowledged"]}'],
                              '[AFSPLUS-KILL] VERIFIED')
            match = re.search(r'VERIFIED generation=(\d+) acknowledged=(\d+)', recovered)
            require(match and case['acknowledged'] <= int(match[1]) <= case['last_attempted'],
                    'recovered generation outside acknowledged/attempted bounds')
            phase(f'{name}-after', ['verify-after'], '[AFSPLUS-KILL] AFTER-PERSISTED')
            checked = subprocess.run([str(args.checker.resolve()), str(image), '--json'],
                                     capture_output=True, text=True, timeout=args.timeout)
            (output / f'{name}-check.json').write_text(checked.stdout)
            require(checked.returncode == 0 and json.loads(checked.stdout)['clean'],
                    'independent checker rejects recovery')
            case['recovered_generation'] = int(match[1])
            report['cases'].append(case)
        report['result'] = 'PASS'
    except Exception as error:
        report['error'] = str(error)
        raise
    finally:
        (output / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report))


if __name__ == '__main__':
    main()
