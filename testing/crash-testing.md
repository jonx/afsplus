# Crash and Power-Failure Testing

## Goal

Prove that every acknowledged transaction recovers to a valid state and every unacknowledged transaction is either absent or safely replayable according to the durability contract.

## Fault model

The block backend supports:

- drop write N
- tear write N at sector boundary
- reorder writes where flush barriers do not forbid it
- fail flush
- crash immediately after each write/flush

## Workloads

- create
- mkdir
- append
- overwrite
- truncate
- rename same directory
- rename across directories
- atomic replace
- hard link
- unlink open file
- xattr update
- allocation-region transition
- catalog update
- change-stream append

## Validation after every crash

1. open in NO_CHANGES mode
2. validate superblock candidates
3. simulate recovery
4. run full invariant checker
5. verify user-visible state is one of the allowed transactional outcomes

No "fsck fixed it" outcome is considered sufficient for ordinary journal guarantees.
