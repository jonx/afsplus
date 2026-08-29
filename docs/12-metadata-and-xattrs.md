# 12. Metadata and Extended Attributes

## 1. Core versus extensible metadata

Frequently required fields stay in the core object record.

Less common metadata uses typed attributes.

This avoids repeatedly expanding the core on-disk object format.

## 2. Attribute naming

Namespaces are explicit.

Examples:

```text
aros.comment
aros.icon
aros.mime
user.project
security.*
system.*
```

The exact standardized registry belongs in the feature registry.

## 3. AROS comments

File comments are first-class AROS semantics.

They may be stored using the attribute subsystem, but the AROS handler exposes them through existing comment APIs without requiring applications to know about xattrs.

## 4. Protection bits

Protection bits are core metadata because they are frequently accessed and important for compatibility.

## 5. Icons

AFS+ should permit a future `aros.icon` attribute.

A compatibility layer may synthesize classic `.info` behavior, but no such behavior is required for format epoch 1.

The attribute architecture should make this possible without adding another fixed field.

## 6. Size limits

Attributes have explicit per-value and per-object limits exposed as filesystem capabilities.

Large arbitrary user data should be stored as files, not abused as attributes.

## 7. Storage

Small attributes may be stored near the object record.

Larger attribute sets spill into separate checksummed metadata blocks.

## 8. Unknown attributes

Unknown user attributes are preserved.

Unknown system attributes are governed by their owning feature's compatibility class.
