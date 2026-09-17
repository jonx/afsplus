# Proposed ADR: metadata inheritance of CloneFile and CloneRange

> **ADRs:** [ADR-027](../adr/ADR-027-reflink-clones.md), [ADR-061](../adr/ADR-061-shared-extent-references.md), [ADR-065](../adr/ADR-065-persistent-data-update-policy.md) · **Spec:** [invariants](../spec/invariants.md) ·
> **Tests:** [shared extents](../testing/shared-extents-qualification.md), [fuzzing](../testing/fuzzing.md) · **Milestones:** M07, M14

Target on acceptance: a numbered ADR in `adr/`, closing Q14 in
[open questions](../implementation/open-questions.md); the table below lands
in [clone semantics](../docs/32-reflink-clone-semantics.md) and in the clone
capability of [FS API v2](../docs/13-filesystem-api-v2.md).

Decisions requested: D1 (CloneFile destination), D2 (CloneRange
destination), D3 (the source), D4 (the explicit-preservation option).

## Context

Four sources described the metadata of a clone and none decided it:
[clone semantics](../docs/32-reflink-clone-semantics.md) defer to the API
contract, FS API v2 is silent, [ADR-027](../adr/ADR-027-reflink-clones.md)
says "independent metadata", and
[docs/30](../docs/30-portable-security-model.md#13-move-copy-and-clone-security-semantics)
says the destination inherits the destination's security policy. The
executable `CloneFile` copied the protection word and the modification time,
set creation and change time to the clone time and the link count to one, and
left the data-update policy at full COW
([ADR-065](../adr/ADR-065-persistent-data-update-policy.md)).

Two precedents frame the choice. A Linux `FICLONE` shares data into a file
the caller already created, so every metadata field is the destination's own.
A macOS `clonefile()` and an AmigaDOS `Copy CLONE` create the destination and
carry the content's date and the protection bits. AFS+ `CloneFile` creates
the destination, and `CloneRange` writes into an existing one, so each
follows the precedent of its shape.

The consumer that decides the modification time is a build cache.
Cargo-style fingerprints and editor indexers compare modification times to
skip work; a clone that reports the clone time as its content time forces a
rebuild of bytes that did not change, which removes the reason to clone a
source tree or a package cache ([docs/31](../docs/31-extreme-workloads.md)).

## Decision

D1. `CloneFile` creates a new object:

| Field | Destination receives |
|---|---|
| Object ID, content generation | New |
| Link count | One |
| Size and data mapping | The source's, shared |
| Modification time | The source's: it dates the content, and the content is the source's |
| Creation time, change time | The clone time |
| Protection word | The source's: a cloned executable stays executable, a cloned script stays a script |
| Data-update policy flag | Cleared: full COW ([ADR-065](../adr/ADR-065-persistent-data-update-policy.md)); in-place updates of a shared run are excluded by [ADR-062](../adr/ADR-062-explicit-hybrid-data-updates.md) |
| Security descriptor | None: a descriptor has one owner, and the destination's security comes from the destination's policy ([security preservation container](adr-security-preservation-container.md)) |
| Extended attributes and comment, once they exist | Undecided here; see the open list |

D2. `CloneRange` is a content write to an existing destination. The
destination's modification and change time become the clone time and its
content generation advances. Its creation time, protection word, link count,
data-update policy and security descriptor stay its own. Nothing but bytes
crosses from the source.

D3. The source of either operation keeps every user-visible field, including
its change time. Marking its runs as shared is a layout fact. A clone needs
read access to the source only, and an operation open to a reader must leave
no trace in the source's metadata: a moved change time makes every
incremental backup and every scanner revisit a file whose bytes, size,
protection and links are unchanged.

D4. An explicit option to carry the source's security descriptor into the
clone, for an authorized caller, is an API-level addition with its own
capability bit. The default never carries it.

## Implementation state

D1 and D2 matched the executable behaviour. D3 did not: both operations set
the source's change time to the clone time whenever they rewrote the source
record. They now leave it, and
`crates/afsplus-check/tests/clone_metadata.rs` pins all three decisions with
literal times: a source created at 10, written at 20, changed at 33, with
protection `0x5a`, the in-place policy, a descriptor and two links; a clone
at 40 that reports created 40, modified 20, changed 40, protection `0x5a`,
one link, full COW and no descriptor; a range clone at 50 that moves only
the destination's modification and change time; and a source that reads
10, 20, 33 after both. Before the correction the two source assertions
failed with 40 and 50 in the change time, which is the negative control.

## Compatibility

No disk field, flag or feature changes. The change time of a clone source is
the one observable difference, and no released consumer exists.

## API contract consequences

For the Stage C API work; this proposal edits no API document.

- The clone capability of FS API v2 states D1 to D3 as its metadata contract,
  and a filesystem-neutral test asserts them through the API.
- A handler or VFS layer that emulates a clone by create, copy and
  set-attributes on a volume without shared extents produces the D1 result,
  so an application sees one contract.
- D4 reserves one capability bit and one request flag; neither is assigned
  here.

## Consequences

- The generated namespace family of [fuzzing](../testing/fuzzing.md) already
  models size, bytes, protection and one link for a clone; its oracle now
  rests on a decision.
- Build caches and indexers keep their fingerprints across a cloned tree.
- A clone of a descriptor-protected file is a file without a descriptor whose
  protection word is the source's. A host that evaluates descriptors applies
  its own creation policy to the clone, exactly as for a byte copy.

## Open after this decision

- Extended attributes and the AROS comment: neither has a representation
  yet ([docs/12](../docs/12-metadata-and-xattrs.md)). The `Copy CLONE`
  precedent suggests user attributes and the comment travel and system
  attributes follow their owning feature; the decision belongs with the
  attribute format.
- Inheritance of a parent directory's default security at creation: an
  evaluation-format question under Q5.
- `CloneTree`: stays experimental under Q8.
