# Official Tools

## mkafsplus

Creates volumes and chooses compatibility profile.

Must print:
- UUID
- block size
- region size
- enabled features
- compatibility profile

## afsplus-info

Read-only inspector.

Default behavior must perform zero writes.

## afsplus-check

Verifier and repair tool.

Modes:
- check only
- repair with confirmation
- scripted repair policy
- NO_CHANGES forensic report

## afsplus-resize

Initial implementation priority:
1. grow
2. minimum-size query
3. shrink after block relocation is proven

## afsplus-dump

Structured metadata dump for debugging.

Prefer JSON output option.

## afsplus-catalog

Commands:
- info
- validate
- rebuild
- benchmark

## afsplus-fuse

Host mount front-end using the same portable core.
