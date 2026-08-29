# Incremental Indexing Example

First run:

```text
enumerate all objects
build index
save current change sequence = 500000
```

Later:

```text
GetChangesSince(500000)
```

If history is available:

```text
500001 MODIFY object 42
500002 CREATE object 99
500003 RENAME object 12
```

Apply changes and save new sequence.

If history expired:

```text
FSV2_ERR_RESCAN_REQUIRED
```

Perform a full enumeration and establish a new baseline.
