//! Every `ArosError` against `native/aros/tests/error_numbers.h`, the list the
//! C side checks against the target's `<dos/dos.h>`.

use afsplus_aros::ArosError;

/// The header constant of a variant. Exhaustive on purpose: a new variant
/// does not compile until it is named here, and then needs its line in the
/// list.
fn header_name(error: ArosError) -> &'static str {
    match error {
        ArosError::Unknown => "ERROR_UNKNOWN",
        ArosError::NoFreeStore => "ERROR_NO_FREE_STORE",
        ArosError::BadNumber => "ERROR_BAD_NUMBER",
        ArosError::ObjectInUse => "ERROR_OBJECT_IN_USE",
        ArosError::ObjectExists => "ERROR_OBJECT_EXISTS",
        ArosError::DirectoryNotFound => "ERROR_DIR_NOT_FOUND",
        ArosError::ObjectNotFound => "ERROR_OBJECT_NOT_FOUND",
        ArosError::ObjectTooLarge => "ERROR_OBJECT_TOO_LARGE",
        ArosError::ActionNotKnown => "ERROR_ACTION_NOT_KNOWN",
        ArosError::InvalidComponentName => "ERROR_INVALID_COMPONENT_NAME",
        ArosError::InvalidLock => "ERROR_INVALID_LOCK",
        ArosError::ObjectWrongType => "ERROR_OBJECT_WRONG_TYPE",
        ArosError::DiskWriteProtected => "ERROR_DISK_WRITE_PROTECTED",
        ArosError::DirectoryNotEmpty => "ERROR_DIRECTORY_NOT_EMPTY",
        ArosError::SeekError => "ERROR_SEEK_ERROR",
        ArosError::CommentTooBig => "ERROR_COMMENT_TOO_BIG",
        ArosError::DiskFull => "ERROR_DISK_FULL",
        ArosError::WriteProtected => "ERROR_WRITE_PROTECTED",
        ArosError::NotDosDisk => "ERROR_NOT_A_DOS_DISK",
        ArosError::NoMoreEntries => "ERROR_NO_MORE_ENTRIES",
        ArosError::IsSoftLink => "ERROR_IS_SOFT_LINK",
        ArosError::RecordNotLocked => "ERROR_RECORD_NOT_LOCKED",
        ArosError::LockCollision => "ERROR_LOCK_COLLISION",
    }
}

const ALL: [ArosError; 23] = [
    ArosError::Unknown,
    ArosError::NoFreeStore,
    ArosError::BadNumber,
    ArosError::ObjectInUse,
    ArosError::ObjectExists,
    ArosError::DirectoryNotFound,
    ArosError::ObjectNotFound,
    ArosError::ObjectTooLarge,
    ArosError::ActionNotKnown,
    ArosError::InvalidComponentName,
    ArosError::InvalidLock,
    ArosError::ObjectWrongType,
    ArosError::DiskWriteProtected,
    ArosError::DirectoryNotEmpty,
    ArosError::SeekError,
    ArosError::CommentTooBig,
    ArosError::DiskFull,
    ArosError::WriteProtected,
    ArosError::NotDosDisk,
    ArosError::NoMoreEntries,
    ArosError::IsSoftLink,
    ArosError::RecordNotLocked,
    ArosError::LockCollision,
];

#[test]
fn every_error_number_is_the_one_the_dos_header_states() {
    let list = include_str!("../../../native/aros/tests/error_numbers.h");
    let pinned: Vec<(&str, i32)> = list
        .lines()
        .filter_map(|line| line.strip_prefix("AFSPLUS_DOS_ERROR("))
        .map(|rest| {
            let (name, number) = rest.trim_end_matches(')').split_once(", ").unwrap();
            (name, number.parse().unwrap())
        })
        .collect();
    assert_eq!(pinned.len(), ALL.len(), "one line per variant");
    for error in ALL {
        let name = header_name(error);
        let line = pinned
            .iter()
            .find(|(pinned_name, _)| *pinned_name == name)
            .unwrap_or_else(|| panic!("{name} has no line in error_numbers.h"));
        assert_eq!(error.io_error(), line.1, "{name}");
    }
    // No two variants share a line, so the list has no unused entry either.
    let mut names: Vec<&str> = ALL.into_iter().map(header_name).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), ALL.len());
}
