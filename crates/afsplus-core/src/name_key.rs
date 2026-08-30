//! Versioned, locale-independent directory comparison keys.

use afsplus_format::ident::{Identification, NameKeyAlgorithm};
use afsplus_format::tree::MAX_TREE_KEY_BYTES;
use afsplus_format::{validate_name, FormatError};
use caseless::Caseless;
use unicode_normalization::UnicodeNormalization;

use crate::CoreError;

/// The shared tree wire format reserves four key bytes per maximum input-name
/// byte. A valid name whose normalized key ever exceeded that explicit format
/// bound is rejected instead of being truncated or published inconsistently.
pub const COMPARISON_KEY_MAX_BYTES: usize = MAX_TREE_KEY_BYTES;

pub fn comparison_key(ident: &Identification, name: &[u8]) -> Result<Vec<u8>, CoreError> {
    validate_name(name).map_err(CoreError::InvalidName)?;
    let text =
        core::str::from_utf8(name).map_err(|_| CoreError::InvalidName(FormatError::InvalidUtf8))?;
    let key = match ident.name_key_algorithm {
        NameKeyAlgorithm::LegacyIdentity => name.to_vec(),
        NameKeyAlgorithm::UnicodeNfc => text.nfc().collect::<String>().into_bytes(),
        NameKeyAlgorithm::UnicodeNfcCasefold => text
            .nfd()
            .default_case_fold()
            .nfc()
            .collect::<String>()
            .into_bytes(),
    };
    if key.len() > COMPARISON_KEY_MAX_BYTES {
        return Err(CoreError::InvalidName(FormatError::Overflow(
            "directory comparison key",
        )));
    }
    Ok(key)
}

pub fn validate_entry_key(
    ident: &Identification,
    key: &[u8],
    name: &[u8],
) -> Result<(), CoreError> {
    if key != comparison_key(ident, name)? {
        return Err(CoreError::Corrupt(
            "directory comparison key does not match original name".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use afsplus_format::crc32c::CHECKSUM_CRC32C;
    use afsplus_format::ident::{
        FeatureFlags, Identification, NameKeyAlgorithm, UNICODE_VERSION_16_0_0,
    };
    use afsplus_format::DEFAULT_BLOCK_SHIFT;

    use super::*;

    fn ident(algorithm: NameKeyAlgorithm) -> Identification {
        Identification {
            uuid: [1; 16],
            block_shift: DEFAULT_BLOCK_SHIFT,
            checksum_algorithm: CHECKSUM_CRC32C,
            region_size: 256,
            log_slots: 0,
            features: FeatureFlags::default(),
            name_key_algorithm: algorithm,
            unicode_version: if algorithm == NameKeyAlgorithm::LegacyIdentity {
                [0, 0, 0]
            } else {
                UNICODE_VERSION_16_0_0
            },
            total_blocks: 1024,
            checkpoint_slots: [1, 2],
            metadata_start: 9,
            label: "names".into(),
        }
    }

    #[test]
    fn dependency_tables_match_the_pinned_format_version() {
        assert_eq!(unicode_normalization::UNICODE_VERSION, (16, 0, 0));
        assert_eq!(caseless::UNICODE_VERSION, (16, 0, 0));
    }

    #[test]
    fn sensitive_keys_normalize_but_do_not_fold() {
        let ident = ident(NameKeyAlgorithm::UnicodeNfc);
        assert_eq!(
            comparison_key(&ident, "Cafe\u{301}".as_bytes()).unwrap(),
            "Café".as_bytes()
        );
        assert_ne!(
            comparison_key(&ident, "Résumé".as_bytes()).unwrap(),
            comparison_key(&ident, "RÉSUMÉ".as_bytes()).unwrap()
        );
    }

    #[test]
    fn insensitive_keys_fold_unicode_and_preserve_no_locale_state() {
        let ident = ident(NameKeyAlgorithm::UnicodeNfcCasefold);
        assert_eq!(
            comparison_key(&ident, "Cafe\u{301}".as_bytes()).unwrap(),
            comparison_key(&ident, "CAFÉ".as_bytes()).unwrap()
        );
        assert_eq!(
            comparison_key(&ident, "Straße".as_bytes()).unwrap(),
            comparison_key(&ident, "STRASSE".as_bytes()).unwrap()
        );
        assert_ne!(
            comparison_key(&ident, "I".as_bytes()).unwrap(),
            comparison_key(&ident, "ı".as_bytes()).unwrap(),
            "the filesystem uses Unicode default, never Turkic locale folding"
        );
    }
}
