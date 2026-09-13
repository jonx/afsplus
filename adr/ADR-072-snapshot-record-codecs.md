# ADR-072: Define experimental snapshot registry and lifetime record codecs

Status: Accepted for the ADR-071 prototype; integration and wire freeze require qualification
Amends: ADR-071

## Context

ADR-071 authorizes the integrated lifetime-ledger experiment. Its storage needs
portable records with explicit validation before adding the registry and ledger
to writable checkpoints. This record defines that first encoding unit.

## Decision

Reserve INCOMPAT bit 2 for `org.aros.afsplus:persistent-snapshots`. A reader
without complete snapshot ownership support rejects that feature before writes.
Assign AFST tree kinds 6 (snapshot registry) and 7 (snapshot lifetime ledger),
both owner zero. Existing tree identities retain their meanings.

Both trees use eight-byte big-endian keys and 32-byte leaf values. Value
integers are little-endian. Registry key zero is the control record: next
snapshot ID in bytes 0..8, with bytes 8..32 reserved zero. IDs start at one;
next-ID equal to UINT64_MAX means allocation is exhausted. IDs never wrap or
reuse. Ordinary registry keys identify views; their values contain captured
checkpoint generation, committed transaction ID and object-map root LBA in
three consecutive u64 fields, followed by eight reserved zero bytes.

Ledger key zero is its control record: next physical scan position and total
retained blocks in two u64 fields, followed by sixteen reserved zero bytes.
Cursor zero begins or wraps a scan. Other keys are physical starts, with values
containing run length, allocation birth and retirement in consecutive u64
fields, followed by eight reserved zero bytes. Retirement zero means live;
otherwise birth < retirement <= the enclosing checkpoint generation.

Decode exact key/value widths before reading fields. Reject nonzero reserved
bytes, zero birth/generation/length, future generations, overflowing or out-of-
volume runs, invalid object-map roots and invalid ID controls. Contextual tree
validation additionally enforces key-zero uniqueness, increasing keys,
nonoverlap, canonical merging, current bitmap ownership, reserved-region/pool
exclusion, registry IDs below next-ID and exact retained totals.

## Integration boundary

This unit defines record codecs and feature/tree identities. Root placement in
checkpoints, creation/deletion transactions and the persistent cursor protocol
must be defined before writer integration. Keep snapshot support out of mount's
supported feature mask until that implementation is qualified. A codec's ability
to read an AFST node grants no ability to mount its owning feature.

Feature-absent volumes preserve their existing encoding. The format change is
incompatible for snapshot-enabled volumes. Old Rust and portable C readers must
explicitly reject the new feature; independent C record parity is an integration
gate before advertising snapshot reads there.

## Conformance, repair and resources

Construct complete checksummed registry/ledger leaf block images from fixed
records. Test exact bytes, both-endian fields, truncated values, nonzero reserved
bytes, lifetime boundaries and arithmetic overflow. Corrupted records yield no
partial decoded state; repair tools report the affected tree/key and require
explicit salvage policy before discarding a retained view.

Each record occupies 48 bytes including its eight-byte key and generic item
header. A 4 KiB AFST leaf holds 84 such records before tree splits. Codecs use
fixed-size arrays and allocate no heap. Integrated ledger split/rebalance,
checkpoint publication, RAM and I/O amplification still need measured gates.
