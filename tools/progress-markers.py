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
    if status.startswith('Ongoing'):
        return None
    if status.startswith('Not started'):
        return 0
    if status.startswith('Complete —') or status == 'Complete':
        return 2
    return 1


def label(token, states):
    value = states[token]
    return token if value in (None, 0) else (rf'\[{token}\]' if value == 1 else f'~~{token}~~')


ITEM = re.compile(r'^(- )(.*) <!-- progress: ([a-z0-9-]+) -->$', re.M)


def item_states(text):
    result = {}
    active = False
    for line in text.splitlines():
        if line.startswith('| Item | Status | Evidence |'):
            active = True
            continue
        if active and not line.startswith('|'):
            active = False
        if not active or line.startswith('|---'):
            continue
        cells = [cell.strip() for cell in line.split('|')[1:-1]]
        if len(cells) != 3 or not re.fullmatch(r'[a-z0-9-]+', cells[0]):
            raise ValueError('Invalid progress item row')
        key, status, evidence = cells
        if key in result:
            raise ValueError(f'Duplicate progress item {key}')
        if status not in ('Complete', 'Partial', 'Not started', 'Ongoing'):
            raise ValueError(f'Invalid progress item status {key}')
        if not re.search(r'\[[^]]+\]\([^)]+\)', evidence):
            raise ValueError(f'Progress item needs evidence/owner link: {key}')
        result[key] = state(status)
    return result


def refresh_items(text, items, seen):
    def replace(match):
        prefix, body, key = match.groups()
        if key not in items or key in seen:
            raise ValueError(f'Unknown or duplicate progress marker {key}')
        seen.add(key)
        if body.startswith('~~') and body.endswith('~~'):
            body = body[2:-2]
        if items[key] == 2:
            body = f'~~{body}~~'
        return f'{prefix}{body} <!-- progress: {key} -->'
    return ITEM.sub(replace, text)


def refresh(root, write=False):
    path = root / 'implementation/milestones.md'
    states = {}
    items = item_states(path.read_text())
    seen_items = set()
    groups = {f'Stage {s}': [] for s in 'ABCDEF'}
    in_milestones = False
    for line in path.read_text().splitlines():
        if line.startswith('| ID | Milestone | Status |'):
            in_milestones = True
            continue
        if in_milestones and not line.startswith('|'):
            in_milestones = False
        if not in_milestones:
            continue
        m = ROW.match(line)
        if not m:
            continue
        cells = line.split('|')
        token = m[1]
        if token in states:
            raise ValueError(f'Duplicate milestone {token}')
        states[token] = state(cells[3].strip())
        for stage in set(re.findall(r'Stage [A-F]', cells[-2])):
            groups[stage].append(states[token])
    for stage, values in groups.items():
        if not values:
            raise ValueError(f'No contributing milestones for {stage}')
        finite = [v for v in values if v is not None]
        states[stage] = (2 if all(v == 2 for v in finite) else (1 if any(finite) else 0)) if finite else None
    # Stage 0 has no numbered milestone; its ongoing source review is explicit.
    states['Stage 0'] = None
    stale = []
    pending = {}
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
        if name in ('ROADMAP.md', 'implementation/implementation-plan.md'):
            updated = refresh_items(updated, items, seen_items)
        if name == 'implementation/milestones.md':
            updated = '\n'.join(ROW.sub(lambda m: '| ' + label(m[1], states) + ' |', line)
                                for line in updated.splitlines()) + '\n'
        if name == 'ROADMAP.md':
            updated = re.sub(rf'^(## )({DECORATED})(:)',
                             lambda m: m[1] + label(re.search(TOKEN, m[2])[0], states) + m[3],
                             updated, flags=re.M)
        if updated != text:
            stale.append(name)
            pending[p] = updated
    if set(items) != seen_items:
        raise ValueError('Progress items have no list entry: ' + ', '.join(sorted(set(items) - seen_items)))
    if write:
        for p, updated in pending.items():
            p.write_text(updated)
    return stale


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--write', action='store_true')
    args = parser.parse_args()
    changed = refresh(Path(__file__).resolve().parents[1], args.write)
    print('progress-markers: ' + (', '.join(changed) if changed else 'up to date'))
    raise SystemExit(0 if args.write or not changed else 1)
