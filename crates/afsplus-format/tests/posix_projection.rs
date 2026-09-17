use afsplus_format::posix::{
    mode_of, protection_for_mode, with_mode, MODE_PERMISSION_BITS, MODE_STICKY,
    PROTECTION_AMIGA_ONLY,
};
use afsplus_format::FormatError;

/// Every mode the projection accepts survives a write and a read unchanged.
/// This is the property the FUSE path depends on: `chmod 600` then `stat`
/// must answer 600, for all of them, not for the ones anybody thought to try.
#[test]
fn every_permitted_mode_round_trips_through_the_protection_word() {
    for mode in 0..=MODE_PERMISSION_BITS {
        if mode & MODE_STICKY != 0 {
            continue;
        }
        let word = protection_for_mode(mode).expect("mode without sticky is representable");
        assert_eq!(
            mode_of(word),
            mode,
            "mode {mode:#o} came back as {:#o} through word {word:#034b}",
            mode_of(word)
        );
    }
}

/// The word is not replaced, it is edited. ARCHIVE, PURE and SCRIPT belong to
/// AmigaDOS and a `chmod` from macOS must not be able to clear them.
#[test]
fn the_amiga_only_bits_survive_every_mode_write() {
    for keep in 0..=0xFu32 {
        let existing = keep << 4;
        for mode in [0o000u16, 0o644, 0o755, 0o600, 0o444, 0o777, 0o4755, 0o2755] {
            let word = with_mode(existing, mode).expect("representable");
            assert_eq!(
                word & PROTECTION_AMIGA_ONLY,
                existing & PROTECTION_AMIGA_ONLY,
                "mode {mode:#o} disturbed the Amiga-only bits of {existing:#010x}"
            );
            assert_eq!(mode_of(word), mode);
        }
    }
}

/// Unassigned bits are not the projection's to touch either: a future meaning
/// for one of them must not be erased by a chmod today.
#[test]
fn bits_the_projection_does_not_speak_for_are_left_alone() {
    let untouched = (1u32 << 16) | (1u32 << 23) | (1u32 << 29);
    let word = with_mode(untouched, 0o640).expect("representable");
    assert_eq!(word & untouched, untouched);
    assert_eq!(mode_of(word), 0o640);
}

/// Sticky is refused by name. The project refuses, it does not ignore: a
/// caller that asked for a restriction and got silence would believe it held.
#[test]
fn the_sticky_bit_is_refused_and_never_dropped_in_silence() {
    for mode in [MODE_STICKY, 0o1777, 0o1644] {
        match with_mode(0, mode) {
            Err(FormatError::Invalid(reason)) => {
                assert!(
                    reason.contains("sticky"),
                    "the refusal must name the bit, got {reason:?}"
                );
            }
            other => panic!("sticky {mode:#o} was not refused: {other:?}"),
        }
    }
    // And it is refused rather than masked away: the neighbouring modes work.
    assert!(with_mode(0, 0o777).is_ok());
    assert!(with_mode(0, 0o2777).is_ok());
}

#[test]
fn a_mode_outside_the_permission_bits_is_refused() {
    assert!(matches!(
        with_mode(0, 0o10000),
        Err(FormatError::Invalid(_))
    ));
}

/// The owner nibble is inverted and the other two are not. Stated as literal
/// words rather than derived, so a sign flip in the implementation cannot
/// agree with a sign flip in the test.
#[test]
fn the_owner_nibble_is_inverted_and_the_others_are_not() {
    // A word of all zeros: owner may do everything, group and other nothing.
    assert_eq!(mode_of(0x0000_0000), 0o700);
    // Owner READ set means owner MAY NOT read.
    assert_eq!(mode_of(0b1000), 0o300);
    // Group READ set means group MAY read.
    assert_eq!(mode_of(1 << 11), 0o740);
    // Other READ set means other MAY read.
    assert_eq!(mode_of(1 << 15), 0o704);
    // 0o644 spelled out: owner rw (EXECUTE set = no x), group r, other r.
    assert_eq!(protection_for_mode(0o644).unwrap(), 0b1000_1000_0000_0010);
}

/// Owner `w` covers truncation and unlink, which AmigaDOS splits. Removing it
/// must protect both, and granting it must grant both, or the POSIX view and
/// the classic view disagree about whether a file can be destroyed.
#[test]
fn owner_write_moves_the_delete_bit_with_it() {
    let writable = protection_for_mode(0o600).unwrap();
    assert_eq!(writable & 0b0101, 0, "write and delete are both allowed");
    let read_only = protection_for_mode(0o400).unwrap();
    assert_eq!(read_only & 0b0101, 0b0101, "both are denied");
    // A word where AmigaDOS allows writing but forbids deletion has no POSIX
    // spelling; the projection reports the safe answer, not the flattering
    // one.
    let write_but_no_delete = 0b0001;
    assert_eq!(mode_of(write_but_no_delete) & 0o200, 0);
}

#[test]
fn group_and_other_write_move_their_delete_bits_too() {
    let word = protection_for_mode(0o666).unwrap();
    assert_ne!(word & (1 << 10), 0, "group write");
    assert_ne!(word & (1 << 8), 0, "group delete follows group write");
    assert_ne!(word & (1 << 14), 0, "other write");
    assert_ne!(word & (1 << 12), 0, "other delete follows other write");
    assert_eq!(mode_of(word), 0o666);
}

#[test]
fn set_user_and_group_id_ride_the_top_two_bits() {
    assert_eq!(protection_for_mode(0o4000).unwrap() & (1 << 31), 1 << 31);
    assert_eq!(protection_for_mode(0o2000).unwrap() & (1 << 30), 1 << 30);
    assert_eq!(mode_of(1 << 31) & 0o4000, 0o4000);
    assert_eq!(mode_of(1 << 30) & 0o2000, 0o2000);
}
