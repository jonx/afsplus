# Shared-Extents Completion Evidence

This report retains the host-side acceptance evidence for the shared-extent
mechanism specified by [ADR-061](../adr/ADR-061-shared-extent-references.md)
and the reflink semantics required by
[ADR-027](../adr/ADR-027-reflink-clones.md). The executable acceptance contract
is [testing/shared-extents-qualification.md](../testing/shared-extents-qualification.md).

## Revisions

The `main` revision `09a2ff1` contains the complete reviewed implementation.
Its relevant history is:

- `32b6940`: accepted ADR-061 contract after review of interval, direct-layout,
  compatibility and rc=2→1 semantics;
- `7edd47f` and `d12e5ed`: fail-closed core/checker corrections, bounded
  reference-tree mutation and the left-boundary canonical-merge regression;
- `ed134fd`: complete shared-reference corruption matrix;
- `51f3e30`: shared lifetime, intent-replay and reuse crash matrices;
- `0d40a36`: atomic byte-range `CloneRange` plus its P2 crash matrix;
- `09a2ff1`: filesystem-neutral VFS capability and operation mapping.

The on-disk format stays experimental until the separate M14 freeze gate.

## Executed gates

The release qualification command is:

```sh
cargo test -p afsplus-check --release \
  --test shared_extents --test shared_clone --test shared_crash
```

It passes 15 checker/oracle tests, 17 functional clone tests and 10 exhaustive
power-cut tests. The repository gate also passes:

```sh
make check
```

That gate checks workspace formatting, all default workspace tests and
features, clippy with warnings denied, and the documentation graph.

## Acceptance coverage

The functional suite covers direct-layout promotion, sparse and unwritten
runs, two- and three-reference transitions, write COW, range replacement,
partial byte boundaries, shared-destination replacement, count
canonicalisation and feature-off refusal without a commit.

The checker independently reconstructs maximal reference intervals from every
live mapping. Its corruption cases cover bad bounds/counts/flags, missing,
extra and wrong records, non-canonical records, feature/root/flag
incongruence, unflagged and direct mappings, shared-tree ownership collisions
and unreadable tree nodes.

The crash suite exercises P1–P10 from the acceptance contract. Every modeled
image must select exactly the allowed pre- or post-generation, pass the full
checker and retain byte-exact contents. Durable shared unlink and
rename-replacement are also cut during intent-log replay and must converge
idempotently on the next mount. In particular, rc=2→1 never quarantines the
survivor, and formerly shared storage becomes reusable only after its final
private owner disappears and ordinary generation quarantine permits reuse.

## Resource and compatibility boundaries

The reference oracle stores interval endpoints rather than an array indexed by
volume blocks, and its hostile-address test uses a large LBA with few
intervals. Runtime reference edits use bounded key-range walks with immediate
neighbours instead of loading the volume-wide reference tree. `CloneRange`
buffers at most two partial data blocks; full blocks are reflinked without data
I/O. The prototype file-layout loader continues to materialise the touched
files' extent vectors, so a streaming extent-map edit remains future resource
work rather than part of this claim.

Profiles without the RO-compatible shared-extents feature remain writable,
do not advertise clone capabilities and return `NotSupported`. Range offsets
with different intra-block alignment and same-file range cloning return an
explicit prototype-limit error; callers can fall back to a physical copy.

This report makes no MacAROS bare-metal, emulator, physical A500 or frozen
epoch-1 format claim. Those remain separate platform and M14 gates.
