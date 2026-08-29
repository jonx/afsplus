# 15. Rust, Zed, Git, and Modern Applications

## 1. Why this matters

AFS+ is intended to support a modern self-hosted AROS development environment.

The filesystem design must therefore accommodate real application behavior rather than only classic desktop workloads.

## 2. Rust/Cargo requirements

Important behaviors:

- 64-bit file sizes
- long UTF-8 names
- large build trees
- many tiny files
- atomic rename
- temporary files
- reliable modification times
- symlinks where crates expect them
- build-script process interaction
- filesystem watchers where available

Optional inline data and directory locality are specifically relevant to Cargo-like source trees.

## 3. Git requirements

Git stresses:

- case sensitivity differences
- atomic file replacement
- lock files
- rename behavior
- timestamps
- huge trees
- file mode/protection translation
- symlinks
- fsync correctness

A per-volume or per-directory case policy allows development volumes to be case-sensitive while preserving traditional AROS behavior elsewhere.

## 4. Zed requirements

A code editor needs:

- scalable directory discovery
- canonicalization
- symlink-safe project boundaries
- stable identity through rename
- low-latency notifications
- persistent catch-up after restart where possible

AFS+ provides the strongest implementation through catalog + change stream, but Filesystem API v2 keeps Zed independent from AFS+.

## 5. Ferail requirements

Ferail benefits from bulk namespace enumeration.

AFS+ `global-catalog` allows a sequential metadata scan rather than recursively opening every directory.

After the initial inventory, `change-stream` permits incremental updates.

Fallback remains correct:

```text
catalog unavailable -> recursive enumeration
change history missing -> full rescan
```

## 6. Application qualification

Release tests include real workflows:

- initialize Git repository
- checkout large repository
- clean and incremental Cargo builds
- Zed-like recursive worktree scan/watch
- Ferail full and incremental indexing
- rename large directory trees
- crash during build/update operations
