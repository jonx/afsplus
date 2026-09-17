//! The POSIX permission projection of the AROS protection word.
//!
//! The protection word stored at object payload offset 68 is the classic
//! AmigaDOS one, and `docs/30-portable-security-model.md` §9 already treats
//! it as the classic projection of a volume's permissions. §10 asks for the
//! POSIX rules and had none; these are them, executable.
//!
//! The bit numbers come from this machine's own
//! `dos/dos.h` (`FIBB_*`) and from
//! `workbench/network/common/lib/net/fibex.h` for the group, other and
//! set-id bits, not from memory of AmigaDOS:
//!
//! ```text
//! 0  DELETE    1  EXECUTE   2  WRITE   3  READ     owner, 0 means ALLOWED
//! 4  ARCHIVE   5  PURE      6  SCRIPT  7  unused   no POSIX meaning
//! 8  GRP_DELETE  9 GRP_EXECUTE 10 GRP_WRITE 11 GRP_READ   1 means allowed
//! 12 OTR_DELETE 13 OTR_EXECUTE 14 OTR_WRITE 15 OTR_READ   1 means allowed
//! 30 SGID      31 SUID
//! ```
//!
//! Two things are deliberate and neither is an oversight.
//!
//! The owner nibble is inverted and the group and other nibbles are not.
//! That is AmigaDOS, stated in `fibex.h` as "Regular RWED bits are 0 ==
//! allowed" and "NOTE: GRP and OTR RWED permissions are 0 == not allowed!".
//!
//! Bits 4 to 7 carry ARCHIVE, PURE and SCRIPT, which POSIX cannot express.
//! A mode write is therefore a read-modify-write of the word and never a
//! replacement, so a file that is a script on the AROS side stays one after
//! `chmod` on the macOS side.
//!
//! A mode round-trips exactly: [`mode_of`] of [`with_mode`] is the mode that
//! went in. The WORD does not round-trip in general, and cannot: a file that
//! is writable but not deletable has no POSIX spelling, so reading it gives
//! `w` clear and writing that back sets both bits. The POSIX view is the
//! projection, and a projection loses what it cannot say.

use crate::FormatError;

/// Bits with no POSIX meaning, preserved byte for byte by [`with_mode`].
pub const PROTECTION_AMIGA_ONLY: u32 = 0xF0;

const OWNER_DELETE: u32 = 1 << 0;
const OWNER_EXECUTE: u32 = 1 << 1;
const OWNER_WRITE: u32 = 1 << 2;
const OWNER_READ: u32 = 1 << 3;

const GRP_DELETE: u32 = 1 << 8;
const GRP_EXECUTE: u32 = 1 << 9;
const GRP_WRITE: u32 = 1 << 10;
const GRP_READ: u32 = 1 << 11;

const OTR_DELETE: u32 = 1 << 12;
const OTR_EXECUTE: u32 = 1 << 13;
const OTR_WRITE: u32 = 1 << 14;
const OTR_READ: u32 = 1 << 15;

const SGID: u32 = 1 << 30;
const SUID: u32 = 1 << 31;

/// The POSIX permission bits, `0o7777` at most. Sticky is not among them.
pub const MODE_PERMISSION_BITS: u16 = 0o7777;
/// POSIX sticky, which the protection word cannot carry.
pub const MODE_STICKY: u16 = 0o1000;
const MODE_SETUID: u16 = 0o4000;
const MODE_SETGID: u16 = 0o2000;

/// Reads the POSIX mode a protection word projects.
///
/// Owner `w` is granted only when WRITE and DELETE are both allowed, because
/// a POSIX writer may truncate and unlink and AmigaDOS splits the two.
pub fn mode_of(protection: u32) -> u16 {
    let mut mode = 0u16;
    // Owner: inverted, so a CLEAR bit grants.
    if protection & OWNER_READ == 0 {
        mode |= 0o400;
    }
    if protection & (OWNER_WRITE | OWNER_DELETE) == 0 {
        mode |= 0o200;
    }
    if protection & OWNER_EXECUTE == 0 {
        mode |= 0o100;
    }
    // Group and other: normal sense, a SET bit grants.
    if protection & GRP_READ != 0 {
        mode |= 0o040;
    }
    if protection & GRP_WRITE != 0 {
        mode |= 0o020;
    }
    if protection & GRP_EXECUTE != 0 {
        mode |= 0o010;
    }
    if protection & OTR_READ != 0 {
        mode |= 0o004;
    }
    if protection & OTR_WRITE != 0 {
        mode |= 0o002;
    }
    if protection & OTR_EXECUTE != 0 {
        mode |= 0o001;
    }
    if protection & SUID != 0 {
        mode |= MODE_SETUID;
    }
    if protection & SGID != 0 {
        mode |= MODE_SETGID;
    }
    mode
}

/// Writes `mode` into `protection`, keeping every bit the projection does not
/// speak for.
///
/// A write of `w` moves the DELETE bit with the WRITE bit in all three
/// classes, so the two views cannot drift into a state where POSIX says
/// writable and AmigaDOS refuses the unlink.
///
/// Sticky is refused rather than dropped: the word has nowhere to put it, and
/// silently discarding a permission bit is how a caller comes to believe a
/// restriction is in force when it is not.
pub fn with_mode(protection: u32, mode: u16) -> Result<u32, FormatError> {
    if mode & MODE_STICKY != 0 {
        return Err(FormatError::Invalid(
            "POSIX sticky bit has no AROS protection representation",
        ));
    }
    if mode & !MODE_PERMISSION_BITS != 0 {
        return Err(FormatError::Invalid(
            "POSIX mode carries bits outside 0o7777",
        ));
    }
    // Everything this projection speaks for is cleared and rebuilt; the rest
    // of the word, ARCHIVE, PURE, SCRIPT and every unassigned bit, is kept.
    let spoken = OWNER_READ
        | OWNER_WRITE
        | OWNER_EXECUTE
        | OWNER_DELETE
        | GRP_READ
        | GRP_WRITE
        | GRP_EXECUTE
        | GRP_DELETE
        | OTR_READ
        | OTR_WRITE
        | OTR_EXECUTE
        | OTR_DELETE
        | SUID
        | SGID;
    let mut word = protection & !spoken;
    // Owner: inverted, so DENY sets the bit.
    if mode & 0o400 == 0 {
        word |= OWNER_READ;
    }
    if mode & 0o200 == 0 {
        word |= OWNER_WRITE | OWNER_DELETE;
    }
    if mode & 0o100 == 0 {
        word |= OWNER_EXECUTE;
    }
    // Group and other: normal sense, so GRANT sets the bit.
    if mode & 0o040 != 0 {
        word |= GRP_READ;
    }
    if mode & 0o020 != 0 {
        word |= GRP_WRITE | GRP_DELETE;
    }
    if mode & 0o010 != 0 {
        word |= GRP_EXECUTE;
    }
    if mode & 0o004 != 0 {
        word |= OTR_READ;
    }
    if mode & 0o002 != 0 {
        word |= OTR_WRITE | OTR_DELETE;
    }
    if mode & 0o001 != 0 {
        word |= OTR_EXECUTE;
    }
    if mode & MODE_SETUID != 0 {
        word |= SUID;
    }
    if mode & MODE_SETGID != 0 {
        word |= SGID;
    }
    Ok(word)
}

/// The word a newly created object gets for `mode`, with no Amiga-only bit
/// set. Creation has no previous word to preserve.
pub fn protection_for_mode(mode: u16) -> Result<u32, FormatError> {
    with_mode(0, mode)
}
