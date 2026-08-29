# Fast Enumeration Example

Application code asks the filesystem layer, not AFS+:

```text
iterator = FSV2_EnumerateObjects(volume)
while next(iterator, record):
    index(record)
```

Possible implementations:

```text
AFS+ with catalog      -> sequential catalog stream
AFS+ without catalog   -> directory/object traversal
NTFS integration       -> MFT-oriented backend
exFAT                  -> optimized directory traversal
FFS                    -> recursive traversal
```

Correctness is identical. Only performance changes.
