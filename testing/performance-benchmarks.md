# Performance Benchmarks

Performance results must always include correctness verification.

## Metadata

- create 1 million empty files
- create 1 million 1 KiB files
- list directory with 1 million entries
- stat 1 million random files
- rename directory containing 1 million descendants

The rename benchmark is specifically expected not to rewrite descendant paths.

## Catalog

- enumerate 4 million records
- enumerate 10 million records
- compare catalog versus recursive traversal
- peak memory while streaming

## Change stream

- apply 1 million changes
- restart consumer
- catch up from saved sequence
- expire history and verify RESCAN_REQUIRED path

## Source workloads

- checkout large Git tree
- Git status
- Cargo clean build
- Cargo incremental build
- editor initial project scan
- editor notification storm
- Ferail full index
- Ferail incremental index

## Large data

- sequential 20 GiB write/read
- random 4 KiB I/O
- sparse 100 GiB file with small allocated ranges

## Resource measurements

Capture:

- wall time
- CPU
- peak RAM
- read bytes
- written bytes
- journal bytes
- metadata bytes
