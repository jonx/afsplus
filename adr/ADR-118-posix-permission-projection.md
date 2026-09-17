# ADR-118: The POSIX mode is a projection of the protection word

Status: Accepted

## Context

A person copied a folder into a mounted AFS+ volume with the Finder. The
Finder said the destination was read-only, made the directory, and failed on
every file inside it. The volume was mounted read-write and shell writes
worked.

From a shell, with no Finder:

```text
python3 -c "import os; os.close(os.open('t.md', os.O_WRONLY|os.O_CREAT, 0o600))"
  -> OSError 102 EOPNOTSUPP "Operation not supported on socket"
chmod 600 f, chmod 755 f, chmod 444 f -> the same 102
cp -R src dst -> "fchmod failed" per file, modes lost
```

`rsync`, `tar`, `zip`, `git init` and making a script executable all failed
the same way, each on a file created with a mode.

The cause was not a permission check. Nothing stored a mode: the FUSE adapter
synthesized one from its mount options, so `setattr` could accept only a mode
equal to the configured one and refused every other with `EOPNOTSUPP`, which
macOS prints with that misleading socket wording and the Finder renders as a
read-only destination.

The same function was handed `atime`, `mtime` and `ctime`, stored none of
them, and answered SUCCESS. `touch -t` returned 0 and the modification time
did not move.

Two candidate carriers existed. A new field could hold a POSIX mode beside the
AROS protection word, or the protection word could carry both. A second field
means two sources of truth for one question and a rule for what happens when
they disagree, which is a rule nobody can test into correctness.

## Decision

The POSIX mode is a PROJECTION of the existing 32-bit AROS protection word at
object payload offset 68, exactly as the AmigaDOS view already is
(`docs/30` section 9). There is one carrier and no second field.

The bit meanings are AmigaDOS's, read from this machine's `dos/dos.h` and
`fibex.h` rather than from memory: owner DELETE, EXECUTE, WRITE, READ at bits
0 to 3 with 0 meaning ALLOWED; ARCHIVE, PURE and SCRIPT at 4 to 6 with 7
unassigned; group at 8 to 11 and other at 12 to 15 with 1 meaning allowed;
SGID and SUID at 30 and 31.

Three rules follow, and `docs/30` section 10.1 states them for readers:

1. Owner `w` requires WRITE and DELETE both allowed, and a `w` write moves
   both, in all three classes. A POSIX writer may truncate and unlink;
   AmigaDOS splits the two. Without this the views drift into one saying
   writable while the other refuses the unlink.
2. Bits 4 to 7 are PRESERVED. A mode write is a read-modify-write of the
   word, never a replacement, so a `chmod` from macOS cannot clear the SCRIPT
   bit of a file AmigaDOS runs.
3. The POSIX sticky bit has no carrier and is REFUSED by name, not dropped.
   At creation the representable part is written and the bit is visibly
   absent, which is what a POSIX filesystem does with a mode bit it cannot
   keep.

A mode round-trips exactly. The WORD does not and cannot: writable but not
deletable has no POSIX spelling. That asymmetry is documented where the
function is defined instead of being discovered later.

The owner UID and GID become real stored fields, `u32` each, at payload
offsets 96 and 100. The fixed payload grows from 96 to 104 bytes and the
security and attribute references move accordingly. This is not a POSIX
favour: the AROS `FileInfoBlock` has `fib_OwnerUID` and `fib_OwnerGID` and
has never had anything to read them from. They are `u32` because POSIX is,
and the AROS projection reports a stored identity that fits in a `UWORD` and
the unknown owner otherwise, rather than truncating one user into another.

A POSIX mode write IS a protection-word write, so it goes through the
projection policy of `docs/30` section 9 unchanged: strict refuses, preserve
marks the projection diverged. No second policy is invented for POSIX.

The modification time a caller names is stored. There is no access time in
this format and none is invented: adapters report the modification time in
its place, which is a declared property of the volume rather than a write
discarded in silence. The metadata-change time is not the caller's to choose.

## Consequences

The format changes, which is authorised: no image has been released, there is
no legacy, and the layout moves in place rather than behind a feature bit.
Every reader moves with it in one commit, Rust and the portable C reader
together, because a second reader that disagrees about where a security
reference lives makes every cross-read test red and hides the next real
breakage among the known ones.

Existing images do not survive the payload change and are recreated. The only
images that existed were development ones.

What this does NOT do: it stores no ACL, evaluates no permission, and gives
the core no opinion about who may do what. The trusted host still decides,
as it did before. A volume carrying a security descriptor keeps the descriptor
as its authority and the word as its projection.

Verified on a real macFUSE mount, not asserted: the reproduction above
succeeds, `cp -R` preserves modes, `touch -t` moves the time, `chmod +x`
makes a script that then runs, and `git init`, `rsync -a`, `tar` and `zip`
complete.
