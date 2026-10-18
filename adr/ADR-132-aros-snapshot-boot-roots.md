# ADR-132: Proposed snapshot boot roots and system update lifetime

Status: Proposed

Related: ADR-061, ADR-071, ADR-073, ADR-074, ADR-125, ADR-128, ADR-131

<!-- toc -->

- [Context](#context)
- [Proposed decision](#proposed-decision)
  - [System roots and mutable data](#system-roots-and-mutable-data)
  - [Adoption into a candidate namespace](#adoption-into-a-candidate-namespace)
  - [Runtime and persistent boot references](#runtime-and-persistent-boot-references)
  - [Publication, selection and fallback](#publication-selection-and-fallback)
- [Required proof before implementation is accepted](#required-proof-before-implementation-is-accepted)
- [Estimate and open decisions](#estimate-and-open-decisions)

<!-- /toc -->

## Context

AROS can mount snapshot-enabled volumes with explicit limits, create and
read snapshots, restore fresh files/trees and capture boot preferences.
The disposable AROS VM run `20261018-afs-snapshot-vm-011` passes its five
probes; the independent SET5 review observes five restored settings through
three live consumers. These proofs do not qualify an installed M1 generation.

SAFE7 needs a different operation: choose an immutable system namespace at
boot, construct the next system without copying its file data, and retain a
working previous boot target through interrupted updates. Current snapshot
readers are read-only. The current fresh-tree restore copies file data and
does not adopt historical extents into a writable namespace. ADR-071 requires
an explicit ownership lifetime rule before such adoption or rollback.

This proposal requires John's separate agreement before implementation.
Writing this proposal gives no permission to activate snapshots, format or
migrate AFSSYS. Each destructive internal-storage operation needs John's
confirmation, after host-image and VM proofs.

## Proposed decision

### System roots and mutable data

Describe a boot root by volume identity, registered snapshot ID and a
validated system-directory object within that snapshot. Bind this identity
to the exact boot bundle and package manifest set. A selected root must be a
reachable directory in the named retained snapshot; a raw object number is
not sufficient admission evidence.

Mount that directory as a read-only ordinary DOS SYS view. Reads, locks,
Examine, ExAll, seeks and executable loading use its pinned snapshot context.
Mutating DOS actions return write protection. Boot-view admission is granted
by the trusted boot selection to ordinary SYS readers; it must not require
the administrative BACKUP authority for every executable load. Administrative
historical-view operations retain ADR-125 authority. Parent, absolute/colon
paths and object-ID operations cannot escape the selected system directory.
The present VIEW command is a historical stream reader and does not implement
this mount.

Keep mutable ENVARC, user homes and service secrets in explicitly selected
writable namespaces outside the selected immutable system directory. Their
state is outside system rollback. Boot Settings capture may use its existing
RAM-copy fallback when SYS is read-only. Verify the complete Startup-Sequence
and every system writer against that boundary before native admission.

### Adoption into a candidate namespace

Create a fresh candidate system directory in the ordinary writable volume,
with its own object identities and namespace metadata. Adopt retained file
runs by shared references, then apply packages into this candidate using COW.
Existing live and snapshot namespaces remain unchanged. Metadata allocations
and bounded traversal are allowed; the copied-file-data counter must be zero.
Publish a registered snapshot only after the complete candidate and its
manifests verify. The boot root names its candidate directory within that
snapshot. No operation replaces the volume's entire live root.

When a historical run becomes ordinarily owned again, preserve its earlier
ownership interval and record the new episode at the adoption generation.
Never move its original birth or erase its prior retirement to fabricate
continuous ownership across a gap. Retention follows the union of actual
ordinary-ownership episodes and registered-view intersections. Ordinary
shared references describe live mappings within their checkpoint; boot/view
retention does not inflate these counts. Retention consults registered views
in both protected selectable checkpoints under ADR-074, including a view
removed from the current registry but still named by the previous one.
Shared references, quarantine and reclaim remain accounted exactly once.
Coalesce only intervals whose ownership is actually continuous.

The persistent episode representation, new reference accounting and checker
rules need a separate format record before writer changes. Readers and
writers that do not implement the feature must reject it before writes.
There is one admitted representation; no silent legacy interpretation or
fallback writer. Measure episode/metadata amplification, candidate size,
edit work and reclaim reserves on the actual system tree before selecting
budgets. Low space refuses admission without expiring a boot root.

### Runtime and persistent boot references

A running SYS view holds a snapshot reader pin until clean handler shutdown.
Deletion, format and handler shutdown with outstanding users retain the
existing busy/write-protection rules. Runtime pins vanish at reboot; the
registered snapshots and persistent boot references remain. Clean shutdown
first stops new namespace users and requires all external locks/readers to
close, then releases the mount-owned root pin before closing the view and
completing ACTION_DIE. An external user still returns busy. The handler cannot
treat its own mount pin as a permanently outstanding external reader.

Add explicit persistent boot references before making an ESP boot descriptor
selectable. Each selectable accepted, trial or fallback descriptor requires
its own matching reference, including both loader slots. A snapshot with a
boot reference cannot be deleted even while it has no runtime reader. This
extends ADR-071 only after this proposal is accepted and the format rule is
specified. No deferred deletion or implicit expiry is introduced.

Drop a boot reference only after its descriptor cannot be selected and its
runtime view has closed. Interrupted publication may retain an extra bounded
reference. Reconciliation must prove descriptor retirement before release;
a missing or corrupt descriptor alone does not authorize reclamation. Bound
candidate/reference counts and measure their reserves; keep the previous
working generation throughout admission failures.

### Publication, selection and fallback

AFS checkpoints and ESP boot records are distinct durable domains. The
protocol must tolerate every cut between them; they are not one transaction:

1. Build and verify the private candidate, then durably register its snapshot
   and persistent boot reference in AFS.
2. Write an inactive-slot boot descriptor binding that root to its exact
   bundle and manifest identities; flush and verify the complete record.
3. Make it a selectable trial generation without retiring the previous one.
4. Decrement its durable attempt budget before entering the candidate kernel.
   Mark it accepted only after the required desktop/health acknowledgement.
5. On exhausted attempts, select the previous verified descriptor and report
   fallback. Remove obsolete descriptors/references only after acceptance and
   the retirement protocol complete.

Use the existing loader generation/slot machinery where its verified
contract applies. Specify any new descriptor or handoff field before changing
that ABI. An unaware loader/kernel must refuse the selection explicitly.
The boot menu offers the retained previous identity without changing a live
volume root or selecting an arbitrary checkpoint.

## Required proof before implementation is accepted

First qualify adoption/accounting on host images: multiple ownership gaps,
shared runs, holes, private unwritten storage, snapshot deletion and branch
retirement, complete remounts and independent checker coverage. Inject every
publication/reclaim cut and admission/allocation failure; compare complete
old/new namespaces, metadata and bytes. Old continuous-lifetime adoption is
a required negative control.

Then qualify ordinary DOS snapshot-root mounts in the VM, including actual
LoadSeg, locked directories, readonly refusal, boot-reference deletion
refusal, mutable ENVARC outside SYS and Startup-Sequence. Require zero copied
file data during candidate creation and bounded measured metadata work.

Exercise at least ten SAFE7 cuts across AFS/ESP publication and acknowledgement,
plus a broken candidate, exhausted attempt budget, and disabled-fallback
negative control. Every accepted boot is the recorded old or new system;
unreachable candidates must never become a third selectable state. Repeat
on an installed matching M1 generation with retained harness artifacts.
VM results do not establish native power-loss, storage or boot behavior.

## Estimate and open decisions

Planning estimate for one engineer, after agreement and prerequisite access:

| Work | Engineering days |
|---|---:|
| Finalize episode/boot-reference format and cross-domain protocol | 1–3 |
| Snapshot-to-candidate adoption, accounting and independent checker | 6–10 |
| Ordinary DOS snapshot-root mounts and mutable-namespace audit | 4–6 |
| Boot descriptor, selection, attempt counter and acknowledgement | 4–6 |
| Host/VM cut harnesses and installed-generation native validation | 5–10 |
| Total | 20–35 |

This is roughly four to seven working weeks, excluding review waits, native
access and any separately authorized volume migration. The episode format,
mutable ENVARC/home placement, reference capacity, acknowledgement authority
and existing loader integration must be resolved before accepting the plan.
No implementation or boot-root experiment is included in the present mission.
