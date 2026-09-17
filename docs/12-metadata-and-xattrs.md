# 12. Metadata and Extended Attributes

> **ADRs:** [ADR-106](../adr/ADR-106-stored-object-comment.md), [ADR-108](../adr/ADR-108-extended-attributes.md) · **Spec:** none ·
> **Tests:** none · **Milestones:** none

## 1. Core versus extensible metadata

Frequently required fields stay in the core object record.

Less common metadata uses typed attributes.

This avoids repeatedly expanding the core on-disk object format.

## 2. Attribute naming

Namespaces are explicit.

A name is 1 to 255 bytes of UTF-8 without NUL. It starts with `user.`,
`system.`, `security.` or `aros.` and has at least one byte after the
namespace ([ADR-108](../adr/ADR-108-extended-attributes.md)). The filesystem
gives no namespace a meaning; access policy belongs to the host adapter.

Examples:

```text
aros.icon
aros.mime
user.project
security.*
system.*
```

The exact standardized registry belongs in the feature registry.

## 3. AROS comments

File comments are first-class AROS semantics.

The comment is a field of the object record ([ADR-106](../adr/ADR-106-stored-object-comment.md)): at most 255 bytes of UTF-8, read with the record on every lookup and directory listing, carried by every rewrite of the record and by `CloneFile`, and captured by snapshots. The AROS handler exposes it through the existing comment APIs. It is independent of the attribute subsystem.

## 4. Protection bits

Protection bits are core metadata because they are frequently accessed and important for compatibility.

## 5. Icons

AFS+ should permit a future `aros.icon` attribute.

A compatibility layer may synthesize classic `.info` behavior, but no such behavior is required for format epoch 1.

The attribute architecture should make this possible without adding another fixed field.

## 6. Size limits

Attributes have explicit per-value and per-object limits exposed as filesystem capabilities: a value holds at most 65,535 bytes, and the encoded set of one object at most 65,536 bytes, names and framing included.

Large arbitrary user data should be stored as files, not abused as attributes.

## 7. Storage

The whole attribute set of an object is one blob, sorted by name, in a chain of checksummed `"AFSA"` blocks the object owns; the object record names the chain with a 16-byte reference behind object flag bit 4 ([ADR-108](../adr/ADR-108-extended-attributes.md)). A set of up to 4,040 bytes costs one 4 KiB block. The chain is immutable: every change, or batch of changes to one object, writes a new chain and retires the old one in the commit that publishes the new record, so a power cut leaves the old set or the new one. `CloneFile` copies the set; deleting the object frees it. An object without attributes has no chain.

A volume with persistent snapshots refuses attributes until the snapshot lifetime ledger owns these chains.

## 8. Unknown attributes

Unknown user attributes are preserved.

Unknown system attributes are governed by their owning feature's compatibility class.
