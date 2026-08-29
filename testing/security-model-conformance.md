# Security Model Conformance and Performance Tests

Status: required test plan before security-format freeze

## 1. Identity round-trip

Verify that stable AFS+ principals survive moves between hosts even when the current host cannot resolve the account.

Cases:

- known local user
- known group
- unmapped foreign user
- unmapped foreign group
- OWNER@ / GROUP@ / EVERYONE@
- deleted local account whose AFS+ principal remains on disk

No test may silently replace an unknown principal with another principal.

## 2. ACL evaluation vectors

Build language-neutral test vectors covering:

- ALLOW only
- DENY before ALLOW
- overlapping user/group memberships
- EVERYONE@ interaction
- owner and owning-group interaction
- READ/WRITE/EXECUTE/TRAVERSE
- DELETE vs DELETE_CHILD
- ACL-management rights

Rust and C implementations must produce identical granted/denied masks.

## 3. Inheritance vectors

Test every combination of:

- file inherit
- directory inherit
- inherit only
- no propagate
- inherited marker
- protected/stop-inheritance descriptor

Include creation, rename, move, copy, reflink clone, and hard-link operations.

## 4. Cross-platform projection

### POSIX adapter

Test:

- simple rwx lossless projection
- named user/group POSIX ACL projection
- ACL containing DENY that POSIX cannot express
- inheritance combinations POSIX cannot express

Expected behavior must distinguish lossless, preserved-but-not-exposed, and degraded mappings.

### Windows adapter

Test canonical ALLOW/DENY, owner, inheritance and protected ACL round trips against equivalent Windows security descriptors where possible.

### Classic Amiga/AROS adapter

Test that protection-bit edits do not accidentally erase richer security descriptors.

## 5. Strict mount behavior

A host that cannot enforce an active security feature must be tested in:

- strict mode: unsafe RW mount/operation rejected
- preserve mode: full metadata retained while the AFS+ layer continues authoritative enforcement where possible
- compat mode: downgrade only after explicit opt-in

## 6. Security descriptor sharing

Create one million files inheriting the same descriptor and measure:

- on-disk metadata overhead
- descriptor lookup CPU
- cache memory
- create latency
- ACL-change cost

Then compare against duplicated per-object ACL storage.

Shared descriptors must behave immutably: modifying one object's ACL must not alter another object's effective ACL unintentionally.

## 7. Security-domain prototype

If security domains are implemented, benchmark:

- policy change over 1 million-object subtree
- normal access-check cost
- cache hit/miss cost
- move between domains
- hard-link constraints across domains
- descriptor override density

Compare against recursively materialized ACL changes.

## 8. Access-check performance

Benchmark hot-path access checks for:

- single-user/classic profile
- simple owner/group/everyone ACL
- 8 ACE ACL
- 32 ACE ACL
- 128 ACE pathological ACL
- many group memberships

Metrics:

- ns/access check on host reference platform
- CPU cycles
- peak/steady memory
- descriptor-cache hit ratio
- allocations per check (target zero on hot path)

## 9. Namespace operation security semantics

Explicitly verify:

```text
rename/move same FS -> security preserved
copy -> destination inheritance by default
CloneFile -> destination inheritance by default
explicit preserve-security copy/clone -> authorized only
```

No operation may accidentally inherit security merely because data extents are shared.

## 10. Raw-media threat test

Demonstrate and document that ACLs alone do not encrypt plaintext data.

When an encrypted security-domain prototype exists, test raw image inspection with keys absent/present separately from ACL enforcement tests.

## 11. Failure injection

Inject failures while:

- creating a new descriptor
- switching an object to a new descriptor
- updating owner
- changing inheritable ACL
- deleting an unused descriptor
- updating security-domain policy

After every simulated crash, objects must reference either the old valid descriptor or the new valid descriptor, never partial security data.

## 12. Benchmark reporting

Every security optimization must report at least:

- CPU
- peak RAM
- descriptor metadata bytes
- block reads/writes
- write amplification
- access-check latency p50/p95/p99

A faster ACL mechanism that materially weakens portability or enforcement fidelity is not an acceptable optimization.
