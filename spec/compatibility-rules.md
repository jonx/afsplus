# Compatibility Rules

## Mount decision algorithm

1. Validate identification record and superblock checksum.
2. Validate format epoch.
3. Read active feature set.
4. For each unknown feature:
   - COMPAT: continue
   - RO_COMPAT: force read-only or fail if RW requested
   - INCOMPAT: fail mount
5. Verify feature dependencies.
6. Apply requested compatibility profile restrictions.
7. If dirty:
   - normal RW: replay journal
   - read-only: follow documented policy
   - NO_CHANGES: never write media
8. Validate authoritative root structures before exposing the volume.

## Tool rule

Repair tools are stricter than normal mounts.

A repair tool must not modify structures controlled by a feature it does not understand.
