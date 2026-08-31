# 30. Portable Multi-User Security Model

> **ADRs:** none · **Spec:** none ·
> **Tests:** none · **Milestones:** none

<!-- toc -->

- [1. Goal](#1-goal)
- [2. Why UID/GID or Windows SID cannot be the format foundation](#2-why-uidgid-or-windows-sid-cannot-be-the-format-foundation)
- [3. Canonical principal identity](#3-canonical-principal-identity)
- [4. Well-known principals](#4-well-known-principals)
- [5. Canonical ACL model](#5-canonical-acl-model)
- [6. Rights](#6-rights)
  - [File/directory data rights](#filedirectory-data-rights)
  - [Metadata rights](#metadata-rights)
  - [Namespace/security rights](#namespacesecurity-rights)
- [7. Inheritance](#7-inheritance)
- [8. Security descriptors as shared objects](#8-security-descriptors-as-shared-objects)
- [9. Classic Amiga compatibility profile](#9-classic-amiga-compatibility-profile)
- [10. POSIX mapping](#10-posix-mapping)
- [11. Windows mapping](#11-windows-mapping)
- [12. Administrator and superuser semantics](#12-administrator-and-superuser-semantics)
- [13. Move, copy, and clone security semantics](#13-move-copy-and-clone-security-semantics)
  - [Rename/move within the same filesystem](#renamemove-within-the-same-filesystem)
  - [Copy](#copy)
  - [Reflink CloneFile](#reflink-clonefile)
- [14. Security domains for subtrees](#14-security-domains-for-subtrees)
- [15. Encryption is a separate protection layer](#15-encryption-is-a-separate-protection-layer)
- [16. Auditing](#16-auditing)
- [17. Security fidelity must be queryable](#17-security-fidelity-must-be-queryable)
- [18. Threat-model principle](#18-threat-model-principle)

<!-- /toc -->

## 1. Goal

AFS+ must preserve useful single-user Amiga semantics while also being able to serve as a serious multi-user filesystem on AROS, Linux, BSD, Windows, macOS, or another operating system.

The on-disk format must not assume that one operating system's account identifiers are universal.

The core rule is:

> AFS+ stores portable principals, ACL semantics, inheritance, and security metadata. Each host operating system maps its local identities and privilege model onto that canonical representation.

Classic systems may expose a simplified view without destroying richer security metadata.

## 2. Why UID/GID or Windows SID cannot be the format foundation

A raw Unix UID such as `1000` has meaning only inside one account database or identity domain.

A Windows SID is richer and globally scoped but is still an operating-system-specific identity representation.

AFS+ therefore must not define file ownership as a native Unix UID/GID pair or as a native Windows SID.

Instead, the filesystem stores principal references that remain stable even when the volume is moved between operating systems.

## 3. Canonical principal identity

A principal reference is conceptually:

```text
principal_kind
security_realm_uuid
principal_uuid
```

The security realm identifies the authority that created/manages the principal namespace.

The principal UUID identifies a user, group, service identity, or other security subject inside that realm.

Hosts maintain mappings such as:

```text
AFS+ principal 9f... -> Unix uid 1000
AFS+ principal 9f... -> Windows SID S-1-5-21-...
AFS+ principal 9f... -> macOS account UUID ...
```

The mappings are host/security-service policy and do not redefine the on-disk identity.

Unmapped principals remain preserved rather than silently converted to `nobody` and discarded.

## 4. Well-known principals

AFS+ should define a small set of portable special principals inspired by NFSv4:

- `OWNER@`
- `GROUP@`
- `EVERYONE@`
- `AUTHENTICATED@`
- `ANONYMOUS@`
- `SYSTEM@` or equivalent trusted-service principal, if the final threat model justifies it

`EVERYONE@` literally includes owner and group. It is not equivalent to Unix `other`.

Administrative/root bypass is deliberately not encoded as an ACL principal. Whether a local administrator or superuser may override discretionary ACLs is a host policy decision.

## 5. Canonical ACL model

The baseline ACL model should be close enough to NFSv4 and Windows ACLs to permit loss-minimized translation.

Each Access Control Entry (ACE) contains:

```text
principal
ACE type
rights mask
inheritance flags
```

Baseline ACE types:

- ALLOW
- DENY

Optional audit profile:

- AUDIT success
- AUDIT failure

ACL ordering and evaluation rules must be fully specified and deterministic.

## 6. Rights

The canonical access mask should distinguish at least:

### File/directory data rights

- READ_DATA / LIST_DIRECTORY
- WRITE_DATA / ADD_FILE
- APPEND_DATA / ADD_SUBDIRECTORY
- EXECUTE / TRAVERSE

### Metadata rights

- READ_ATTRIBUTES
- WRITE_ATTRIBUTES
- READ_XATTR
- WRITE_XATTR

### Namespace/security rights

- DELETE
- DELETE_CHILD
- READ_ACL
- WRITE_ACL
- WRITE_OWNER

The names deliberately resemble NFSv4/Windows rights because those models already solve more cases than simple POSIX rwx bits.

Host-specific rights must not silently enter the disk format. New rights require feature/version negotiation.

## 7. Inheritance

Directories may carry inheritable ACEs.

Required inheritance semantics:

- inherit to files
- inherit to directories
- inherit-only
- no-propagate
- inherited marker
- protected ACL / stop inheritance

The normal baseline is inheritance at object creation time.

Recursive rewriting of millions of existing ACLs when a parent's defaults change is not required by the base model.

A separate security-domain mechanism may later provide efficient subtree policy changes.

## 8. Security descriptors as shared objects

Many files in one directory inherit identical security metadata.

Duplicating a long ACL inside every object wastes space and increases write amplification.

AFS+ should therefore prototype shared immutable security-descriptor objects:

```text
object 42 -> security_descriptor_id 981
object 43 -> security_descriptor_id 981
object 44 -> security_descriptor_id 981
```

A descriptor includes:

- owner principal
- owning-group principal
- discretionary ACL
- audit ACL where supported
- control/inheritance flags
- checksum/generation metadata

Changing one object's ACL creates or references another descriptor rather than mutating shared state unexpectedly.

Descriptor deduplication may be hash/index assisted but must never become a correctness dependency.

## 9. Classic Amiga compatibility profile

Classic Amiga systems must not need a full account service merely to mount AFS+.

The `classic-rw` profile may map the local session to `OWNER@` and expose the traditional Amiga protection bits as a simplified projection.

Example:

```text
AFS+ rich ACL
      |
classic compatibility adapter
      |
AROS/Amiga protection bits
```

Critical rule: a classic implementation may preserve rich ACLs it cannot fully edit or display, but must not silently rewrite them into a weaker representation.

If a requested write would destroy security information the implementation should reject it unless the user explicitly requests security downgrade/conversion.

## 10. POSIX mapping

POSIX rwx and POSIX ACLs are a projection of the richer AFS+ model.

The mapping must follow explicit rules for:

- owner
- owning group
- named users
- named groups
- other/everyone
- default/inherited ACLs

When the AFS+ ACL contains semantics POSIX cannot represent exactly, such as ordered DENY ACEs or certain inheritance combinations, the host must not silently claim a lossless mapping.

Possible host policies:

- `security=strict`: refuse read-write mount or refuse the operation if semantics cannot be enforced
- `security=preserve`: preserve full ACL and expose the closest host view while the AFS+ enforcement layer remains authoritative
- `security=compat`: explicit opt-in downgrade behavior for constrained systems

## 11. Windows mapping

Windows DACLs and inheritance are close to the proposed canonical model.

The adapter should map:

- Windows account SID <-> AFS+ principal
- access-allowed/denied ACEs
- file/container inheritance
- protected inheritance state
- owner/security-descriptor rights

Windows SACL/audit semantics can map to the optional audit profile where implemented.

The adapter must preserve unsupported Windows-specific ACE information only through explicitly versioned extension records, never by pretending the canonical ACL can represent semantics it cannot.

## 12. Administrator and superuser semantics

AFS+ separates discretionary ACL data from local administrative override.

Examples:

- Unix root may be granted an override by the host security layer.
- Windows backup/restore or security privileges may bypass ordinary access checks for specific operations.
- AROS single-user mode may treat the local trusted system context as privileged.

The filesystem format records ACL policy. The operating system defines which privileged contexts may bypass it.

This prevents `administrator` from becoming a magic account hard-coded into removable media.

## 13. Move, copy, and clone security semantics

Security behavior must be explicit because users frequently misunderstand it.

### Rename/move within the same filesystem

Default semantic proposal:

```text
preserve object identity
preserve security descriptor
```

This matches the idea that a move changes namespace location rather than recreating the object.

### Copy

Default semantic proposal:

```text
new object
inherit destination security policy
```

An API may explicitly request security preservation when authorized.

### Reflink CloneFile

Clone creates a new object sharing data extents, not security identity.

Default:

```text
shared data initially
new object ID
new content generation namespace
inherit destination security policy
```

An authorized caller may request cloned security metadata explicitly.

These semantics must be tested across platforms before epoch 1.

## 14. Security domains for subtrees

A future optional feature should prototype named/stable security domains.

Example:

```text
Work:/
  Users/
    Alice/      domain=alice-private
    Bob/        domain=bob-private
  Shared/
    Developers/ domain=dev-team
  System/       domain=system
```

A security domain may provide:

- default/shared ACL policy
- principal/role bindings
- audit policy
- optional quota/accounting association
- optional encryption-key domain in a later feature

Objects can store a `security_domain_id` plus an optional local security descriptor override.

Potential advantage:

Changing a domain policy can affect a huge subtree without rewriting every file's ACL.

However this changes the normal inheritance model and creates questions around cross-domain moves, hard links, snapshots, caching, and offline interoperability. Therefore security domains are Proposed and require prototype evaluation.

## 15. Encryption is a separate protection layer

ACLs protect data only while an operating system honors them.

Someone with raw block access or a host that deliberately ignores ACLs can still read plaintext storage.

Therefore AFS+ must clearly distinguish:

```text
access control
    who may access an object through the filesystem

from

cryptographic confidentiality
    who possesses the key required to decode the stored bytes
```

A future encrypted security-domain feature could allow private subtrees with independent keys, inspired by dataset or directory-key models.

This is not required for the first writable milestone because key lifecycle, recovery, sharing, backup, and portable OS integration require a separate design review.

Also, filesystem encryption should not claim to protect against an active administrator that can access keys while they are loaded unless the threat model explicitly provides hardware or external key isolation.

## 16. Auditing

An optional audit ACL can request security events for selected principals/actions and success/failure outcomes.

Audit events should integrate with the same structured event infrastructure used by content inspection and filesystem observability.

Examples:

```text
user Alice denied READ_DATA on object 42
user Bob changed ACL of directory 17
service scanner accessed object generation 9 using security privilege
```

Audit is a host/security-service function and should not require permanent unbounded logs inside the filesystem itself.

## 17. Security fidelity must be queryable

The mount/API should expose a structured security capability state:

```text
canonical_acl: true
allow_deny: true
inheritance: true
audit_acl: false
principal_mapping: complete
host_enforcement: full
security_degraded: false
```

A host unable to enforce an active security feature must make that fact visible and, in strict mode, refuse unsafe read-write operation.

## 18. Threat-model principle

The highest priority rule is:

> Never silently weaken security merely because the current host has a simpler permission model.

Preserving inaccessible or unknown security metadata is preferable to discarding it.

Failing safely is preferable to mounting read-write with permissions the host cannot enforce.
