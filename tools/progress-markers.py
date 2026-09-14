#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""Generate navigation progress labels from milestone status cells."""
import argparse
from pathlib import Path
import re

TOKEN = r'M\d{2}|Stage [A-F0]'
DECORATED = rf'(?:~~(?:{TOKEN})~~|\\\[(?:{TOKEN})\\\]|(?:{TOKEN}))'
LINK = re.compile(rf'\[({DECORATED})\]\(([^)]+)\)')
ROW = re.compile(r'^\|\s*(?:~~|\\\[)?(M\d{2})(?:~~|\\\])?\s*\|')


def state(status):
    if status.startswith('Not started'):
        return 0
    if status.startswith('Complete —') or status == 'Complete':
        return 2
    return 1


def label(token, states):
    value = states[token]
    return token if value == 0 else (rf'\[{token}\]' if value == 1 else f'~~{token}~~')


def refresh(root, write=False):
    path = root / 'implementation/milestones.md'
    states = {}
    groups = {f'Stage {s}': [] for s in 'ABCDEF'}
    for line in path.read_text().splitlines():
        m = ROW.match(line)
        if not m:
            continue
        cells = line.split('|')
        token = m[1]
        states[token] = state(cells[3].strip())
        for stage in set(re.findall(r'Stage [A-F]', cells[-2])):
            groups[stage].append(states[token])
    for stage, values in groups.items():
        if not values:
            raise ValueError(f'No contributing milestones for {stage}')
        states[stage] = 2 if all(v == 2 for v in values) else (1 if any(values) else 0)
    # Stage 0 has no numbered milestone; its ongoing source review is explicit.
    states['Stage 0'] = 1
    stale = []
    for name in ['README.md', 'ROADMAP.md', 'implementation/README.md',
                 'implementation/implementation-plan.md', 'implementation/milestones.md']:
        p = root / name
        text = p.read_text()
        def replace_link(m):
            token = re.search(TOKEN, m[1])[0]
            return f'[{label(token, states)}]({m[2]})'
        text = re.sub(r'\[(M\d{2}(?:, M\d{2})+)\]\(([^)]+)\)',
                      lambda m: ', '.join(f'[{token}]({m[2]})' for token in m[1].split(', ')), text)
        updated = LINK.sub(replace_link, text)
        if name == 'implementation/milestones.md':
            updated = '\n'.join(ROW.sub(lambda m: '| ' + label(m[1], states) + ' |', line)
                                for line in updated.splitlines()) + '\n'
        if name == 'ROADMAP.md':
            updated = re.sub(rf'^(## )({DECORATED})(:)',
                             lambda m: m[1] + label(re.search(TOKEN, m[2])[0], states) + m[3],
                             updated, flags=re.M)
        if updated != text:
            stale.append(name)
            if write:
                p.write_text(updated)
    return stale


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--write', action='store_true')
    args = parser.parse_args()
    changed = refresh(Path(__file__).resolve().parents[1], args.write)
    print('progress-markers: ' + (', '.join(changed) if changed else 'up to date'))
    raise SystemExit(0 if args.write or not changed else 1)
