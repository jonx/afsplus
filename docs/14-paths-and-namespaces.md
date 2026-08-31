# 14. Paths and Namespaces

> **ADRs:** none · **Spec:** none ·
> **Tests:** [aros-system-volume-qualification](../testing/aros-system-volume-qualification.md) · **Milestones:** M06

## 1. Fundamental separation

AFS+ stores hierarchy and component names.

AROS stores namespace semantics.

AFS+ never stores a full path like:

```text
Work:Projects/Ferail/src/main.rs
```

It stores directory relationships and names:

```text
Projects -> Ferail -> src -> main.rs
```

## 2. Native AROS path model

The modern path layer must understand:

- volume prefixes
- Assigns
- `PROGDIR:`
- relative paths
- parent navigation
- path separators accepted by AROS

These rules belong in `dos.library` or a dedicated path service.

## 3. Rust OsStr/Path

Rust path types are platform-specific by design.

The AROS Rust standard library should implement AROS parsing rules rather than pretending AROS is Unix.

`OsString` must remain capable of losslessly representing names from non-AFS+ filesystems even though AFS+ itself requires valid UTF-8.

A practical representation is a byte sequence plus platform-aware parsing, with UTF-8 guaranteed only when the underlying filesystem declares it.

## 4. POSIX compatibility

Software that hardcodes `/tmp`, `/home`, or other Unix paths cannot be fixed by changing the AFS+ disk format.

A separate compatibility namespace may map conventional POSIX paths onto AROS locations.

Example policy:

```text
/tmp       -> T:
/home/...  -> configured HOME:
/          -> configured compatibility root
```

This is optional compatibility infrastructure.

## 5. Canonicalization

Canonicalization must:

- resolve Assigns according to AROS semantics
- resolve symlinks with loop detection
- return a stable normalized path representation
- permit callers to retrieve object ID separately
- avoid conflating spelling with identity

Security-sensitive applications should compare resolved object/path boundaries, not only textual prefixes.
