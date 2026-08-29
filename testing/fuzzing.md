# Fuzzing

Required fuzz targets:

- identification block
- superblock
- feature table
- object record
- directory node
- extent node
- allocation-region metadata
- journal record
- xattr record
- catalog record
- change-stream record

Properties:

- no crash
- no out-of-bounds access
- no unbounded allocation from corrupt length
- no integer overflow
- deterministic error classification where practical

Seed corpus includes every conformance image plus intentionally corrupt variants.
