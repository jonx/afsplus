# Proposed ADR: reserve an actor field in the change record

> **ADRs:** [ADR-013](../adr/ADR-013-change-stream-bounded.md), [ADR-031](../adr/ADR-031-portable-security-acls.md), [ADR-065](../adr/ADR-065-persistent-data-update-policy.md) · **Spec:** none ·
> **Tests:** [security-scanning-benchmarks](../testing/security-scanning-benchmarks.md) · **Milestones:** M10, M14

Target on acceptance: a numbered ADR in `adr/`; the record section of
[docs/11](../docs/11-change-stream.md) gains the common change-record header
and its actor field, and [Q8](../implementation/open-questions.md) records the
change stream's attribution contract.

Decisions requested: M1 (reserve the field and its layout), M2 (trust level and
format neutrality), M3 (admission rule), M4 (relation to principal identity),
D1 (the API-v2 surface consequence).

<!-- toc -->

- [Context](#context)
- [Decision](#decision)
- [Compatibility classification](#compatibility-classification)
- [Format-change procedure](#format-change-procedure)
- [API contract consequences](#api-contract-consequences)
- [Consequences](#consequences)
- [Open after this decision](#open-after-this-decision)

<!-- /toc -->

## Context

A change record holds an object ID, a sequence number, a record type and the
fields that event needs ([docs/11](../docs/11-change-stream.md)). The stream
therefore answers what changed and never who changed it. On a multitasking
system, bracketing an operation between two sequence numbers yields a window
that contains every concurrent mutation by every other program, which is a
time range rather than an attribution.

The platform register of the enclosing operating-system project lists the
question of whether change records carry an actor among the format decisions
that must land before epoch 1 freezes, and its AFS+ track states the reason: a
change record is persistent on-disk state, so a field added after the freeze is
an epoch change, while the same field reserved before the freeze costs the
record's fixed width and nothing else.

The consumer that needs attribution is the integrity and provenance service
described by that project's integrity-scanner feature. It consumes the change
stream as its incremental scan queue, keyed by object ID and content
generation. A scanner that observes a changed file and cannot say whether a
user action, a packaging tool or an unknown program produced it reports an
event rather than a finding. The packaging feature of the same project does not
need attribution, because a staging tree published atomically attributes its
own contents by construction. One consumer with a stated need and one consumer
with a stated absence of need is the evidence available.

The trust available from the host is bounded and known. On AROS, `DosPacket`
carries `dp_Port`, which `SendPkt()` takes as an argument and stores verbatim;
`struct Message` has no sender field and `PutMsg()` records nothing, so a field
populated from the packet is caller-supplied and forgeable. The mechanism that
would make it evidence is specified in that project's memory-protection feature
as item MP7: `PutMsg()` runs synchronously on the sender's own stack, so the
sending task is observable at that point, and recording it means an Exec-owned
table beside the message, opt-in per port, released on `GetMsg()` or
`ReplyMsg()` so it stays bounded. That item also states that `PutMsg()` is
legal from interrupts, that its own gate requires protected metadata, enforced
caller classification, direct-call bypass rejection and stale-message tests,
and that until those bypass tests pass both lanes expose advisory provenance
only. Native AROS can place the table behind kernel protection; hosted AROS
cannot, because any program in the address space can write it.

Two conclusions follow. The guarantee is a host property that varies by lane
and arrives after the format freezes, so the format cannot promise
authenticity. The value is nevertheless obtainable only while the
change is recorded, so no later reconstruction supplies it.

The security model already names a different concept with a similar shape. A
principal is a canonical security identity, a realm plus a kind plus a UUID,
mapped by each host onto its local identities
([docs/30](../docs/30-portable-security-model.md) section 3). Auditing in
section 16 of that document is a host and security-service function that
reports principals and actions, and it explicitly must not require permanent
unbounded logs inside the filesystem. Debug observability assigns operation,
transaction and checkpoint identities to every mutating operation and traces
them across subsystems including the change stream
([docs/26](../docs/26-debug-observability.md) section 2), and those identities
are runtime, in-memory and not persisted by the stream.

No change-record codec exists. A grep of `crates/` for change-record and
change-stream identifiers returns no encoder, no decoder, no record type and no
test; M10 is not started. The stream's cursor representation and its reset
incarnation token are likewise open design gates
([docs/11](../docs/11-change-stream.md) section 9). Reserving the field
therefore changes no code and no image.

The admission question is settled by precedent rather than by fresh
experiment. The proposed object-record rule admits a record only in its
canonical image, because every mutation path decodes a record into fields and
re-encodes those fields into a zeroed block, so a byte admitted without a field
is destroyed by the next rewrite
([exact admission](adr-object-record-admission.md)). Change records differ on
exactly that point: the stream is append-only, a committed record is never
re-encoded, and retention discards whole records rather than rewriting them
([ADR-013](../adr/ADR-013-change-stream-bounded.md)). The reason for refusing
to tolerate an unknown byte is absent for the payload of the actor field and
present for every byte whose value no writer may choose.

## Decision

M1. The epoch-1 change record carries a fixed 16-byte `actor` field in the
common header that every record type shares, so attribution is uniform across
CREATE, DELETE, RENAME, LINK, UNLINK, DATA_MODIFIED, METADATA_MODIFIED and
XATTR_CHANGED. Its layout is:

```text
actor_class  u16   validated namespace of the host actor identity
actor_trust  u8    the host's claim about the mechanism that produced actor_id
reserved     u8    zero
actor_id     u8[12] opaque to the format, defined by actor_class
```

An all-zero field means unattributed, and every writer that has no actor to
supply writes zero. Twelve opaque bytes are the width that holds the identities
the known hosts can observe without truncation: an AROS Exec-owned sender table
supplies a task identity, a reuse generation and a port or service identity; a
POSIX adapter supplies a pid, a uid and a process start time, which is what
makes a pid non-reusable; a Windows adapter supplies a process and thread pair
with a creation stamp.

M2. The actor field is advisory evidence supplied by the host adapter and is
never authenticated by the format. AFS+ records what the host asserts, verifies
nothing about it, and grants it no authority: no admission, allocation,
protection, repair or mount decision reads the actor field. `actor_trust`
records the host's own claim about its mechanism, with `0` unattributed, `1`
advisory, meaning a value the host could not protect from forgery, and `2`
host-enforced, meaning a value the host obtained from a mechanism it protects.
A trust value is evidence about the host, not proof from the format, and a
consumer that requires proof obtains it from the host's audit service and not
from the stream.

The format stays free of any AROS concept. No task, port, message, `DosPacket`
or `PutMsg` notion appears in the layout or in its documentation: `actor_class`
names a host actor namespace and `actor_id` is bytes. The AROS adapter maps an
Exec-owned sender table into one class, a POSIX adapter maps pid and uid into
another, and neither mapping is format semantics.

M3. Admission separates the bytes a writer may choose from the bytes it may
not:

1. the `reserved` byte is zero, and a nonzero value makes the record corrupt;
2. `actor_trust` holds a value; a reader that does not know the value reports
   it as advisory, the weakest attributed claim, together with the raw value,
   so a later trust level never turns an older reader's verdict into
   corruption and never reads as a stronger claim than the reader can name;
3. when `actor_class` is zero, `actor_trust` and all twelve `actor_id` bytes
   are zero, and any nonzero byte makes the record corrupt;
4. when `actor_class` is assigned, the twelve `actor_id` bytes are that class's
   defined payload and carry no format-level constraint;
5. when `actor_class` is unassigned by the reading implementation, the record
   is admitted and its actor is reported as unreadable, carrying the class
   value and the raw bytes, never a substituted or fabricated actor.

Rules 1 and 3 are the exact-admission rule of
[exact admission](adr-object-record-admission.md) applied to every byte whose
value no writer may choose: reserved bytes are zero or defined and are never
ignored. Rules 2 and 5 depart from that rule deliberately and only for values
a writer chooses and the record itself names. Its justification is the loss
contract that an ignore rule requires, and that contract is satisfiable here:
the stream is non-authoritative and discardable, a committed record is never
re-encoded, so an unrecognised class loses nothing at the next rewrite, and
refusing the whole record instead would deny a consumer the object ID and
sequence number it needs in exchange for a class value it did not ask for.

M4. An actor and a principal are separate fields and separate concepts. An
actor is a runtime subject observed by the host at the time of the change, a
task or a process, whose lifetime ends with that runtime. A principal is a
security identity, stable across reboots and across hosts, that
[docs/30](../docs/30-portable-security-model.md) defines as a realm, a kind and
a UUID. A record answers which running thing performed the change; it does not
answer under which security identity the change was authorized, and an actor is
never mapped to, substituted for, or compared with a principal.

Epoch 1 reserves no principal field in the change record. The security model
places auditing in the host security service and forbids permanent unbounded
logs inside the filesystem, so a principal reference per record duplicates an
authoritative store the format does not own. The extension path that does not
require an epoch change exists: the change stream is an optional feature whose
record types are negotiated, so a later security-enforcing host adds an
attribution record type or an `actor_class` whose payload encodes a host
principal handle, both gated by a feature identity under the
[feature framework](../docs/09-feature-framework.md). That path is unavailable
for the actor itself, because the actor applies to every record type at once
and a parallel record per event would break the one-record-per-committed-event
relation that retention and the sequence contract depend on.

## Compatibility classification

No feature identity is allocated by this decision. The change stream is an
optional feature, `org.aros.afsplus:change-stream`, whose record layout is not
frozen and whose implementation is not started. No writer emits a change
record, no image in the conformance corpus contains one, and no reader decodes
one, so every existing image, fixture and corpus entry stays valid and no
verdict changes. A host that populates the field allocates an `actor_class`
value in the registry that M10 creates; allocating a class is not a format
change and needs no feature bit, because rule 5 gives an implementation that
does not know the class a defined outcome.

## Format-change procedure

| Step | State |
|---|---|
| Specification update | The common change-record header and the actor field land in docs/11 on acceptance; the wire offsets join the disk layout when M10 defines the stream encoding |
| ADR | This proposal |
| Compatibility classification | None needed: no writer, image or corpus entry contains a change record |
| Conformance image | None available: the shared corpus gains change-stream images when the M10 codec produces the first one |
| Parser tests | None exist and none are written here, because no change-record codec exists in `crates/`. The M10 change-stream lot owns the codec and its admission tests: a nonzero reserved byte, an unassigned `actor_trust` reported as advisory with its raw value, a zero class with each of the thirteen following bytes nonzero in turn, a canonical unattributed control that decodes to the literal record and re-encodes to the identical bytes, an attributed control per assigned class, and an unassigned-class record admitted with an unreadable actor |
| Repair-tool behavior | A record that fails rule 1 or rule 3 is reported as a corrupt change record. The checker never repairs it by zeroing the field, because that would assert an unattributed change that the volume does not record. The stream is discardable, so the sanctioned recovery is discard and reset with `FSV2_ERR_RESCAN_REQUIRED` to every cursor ([ADR-013](../adr/ADR-013-change-stream-bounded.md)) |
| Resource impact | Sixteen bytes per record, present whether or not a host supplies an actor. The stream is bounded, so the cost is paid in retention depth: a fixed stream budget retains fewer records in proportion to the record width that M10 fixes. No allocation, no additional I/O and no extra pass, because the field lies inside the record the reader already decodes and the checksum already covers |

## API contract consequences

D1. The change-stream iterator named in the API-v2 capability list
([docs/13](../docs/13-filesystem-api-v2.md)) returns the actor with each
record, and its contract states four things: the actor is host-supplied
evidence and not a filesystem authorization fact; an all-zero actor means
unattributed and never means an unknown or default subject; the trust value
belongs to the record and is not upgraded by the reading host; and an
unreadable class is surfaced as such, so an application cannot mistake it for
an absent actor. A consumer that filters by actor treats an unattributed and an
unreadable record as candidates rather than as exclusions, which keeps a
scanner fail-open with respect to coverage and closed with respect to claims.

The API structure and its ABI are not edited by this proposal. The Stage C
session that owns docs/13 and `api/` places the field in the iterator's record
structure as an additive, capability-gated extension, sized for the sixteen
bytes plus whatever reporting the unreadable case requires, and runs the
API-change procedure of [CONTRIBUTING.md](../CONTRIBUTING.md): ABI and
versioning analysis, legacy compatibility impact, capability semantics, Rust
mapping impact and one filesystem-neutral test.

## Consequences

- The integrity and provenance service obtains a cause with each event, at the
  trust level the running host can support, without the filesystem claiming
  more than the host proves.
- The field costs sixteen bytes per record on every volume whose change stream
  is active, including hosts that never populate it. That cost buys the option;
  the alternative price is an epoch change.
- A field that is never authenticated invites being read as though it were.
  The trust value and the API contract exist to make that misreading visible,
  and no filesystem decision consumes the actor, so a forged value corrupts a
  consumer's report and never the volume.
- The two reference implementations share rules 1 to 5 in one admission
  function from the first codec, so they cannot diverge the way the object
  readers did ([ADR-029](../adr/ADR-029-dual-reference-implementations.md)).
- Change records deliberately adopt a weaker admission rule than object
  records for one bounded payload. The difference is justified by append-only
  records, and the justification fails the moment any path rewrites a committed
  change record. No such path is permitted.

## Open after this decision

- The assigned values of the `actor_class` registry, including which class the
  AROS adapter and which class a POSIX adapter receive. No adapter exists, so
  the values would be invented rather than derived; the registry is created
  with the M10 codec.
- Whether `actor_trust` needs values beyond unattributed, advisory and
  host-enforced, in particular a value distinguishing a native lane whose
  sender table sits behind kernel protection from a hosted lane where any
  program can write it. The MP7 gate of the enclosing project has not run its
  bypass, stale-message and interrupt-attribution controls, so the number of
  mechanisms that a host can honestly distinguish is unmeasured. Rule 2 lets
  the assigned set grow without a feature bit.
- The exact byte offset and alignment of the field inside the common header,
  which follows the record encoding that M10 designs and the M14 wire freeze
  fixes.
- Whether an interrupt-time or kernel-originated change is a distinct class or
  a distinct trust value. The answer depends on how MP7 defines its interrupt
  and delegated-service identities, which the gate has not produced.
- Whether retention policy may prefer attributed records over unattributed ones
  when discarding. This is stream policy rather than wire format, and it needs
  a measured consumer before it earns a rule.
- Whether `actor_id` bytes are covered by the filesystem's privacy posture when
  a volume moves between hosts, given that a class payload can identify a user
  account. The security model's projection and preservation rules are written
  for security descriptors rather than for observational evidence, and the
  question needs a real multi-host adapter ([ADR-031](../adr/ADR-031-portable-security-acls.md)).
