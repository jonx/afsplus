use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use afsplus_block::{BlockDevice, BlockError};
use afsplus_core::mount::{select_checkpoint, Selection};
use afsplus_core::volume::emergency_headroom_for_volume;
use afsplus_core::CoreError;
use afsplus_format::ident::{
    Identification, NameKeyAlgorithm, COMPAT_DATA_POLICY, INCOMPAT_INTENT_LOG,
    INCOMPAT_INTENT_LOG_DATA_UPDATES, RO_COMPAT_ORPHAN_DIRECTORY, RO_COMPAT_SHARED_EXTENTS,
};
use afsplus_format::DEFAULT_BLOCK_SIZE;

pub const EXIT_OK: u8 = 0;
pub const EXIT_MEDIA: u8 = 1;
pub const EXIT_USAGE_OR_IO: u8 = 2;

#[derive(Debug)]
pub struct Failure {
    pub exit: u8,
    pub id: &'static str,
    pub message: String,
}

impl Failure {
    pub fn usage(message: impl Into<String>) -> Self {
        Self {
            exit: EXIT_USAGE_OR_IO,
            id: "E_USAGE",
            message: message.into(),
        }
    }

    pub fn host_io(message: impl Into<String>) -> Self {
        Self {
            exit: EXIT_USAGE_OR_IO,
            id: "E_HOST_IO",
            message: message.into(),
        }
    }

    pub fn media(id: &'static str, message: impl Into<String>) -> Self {
        Self {
            exit: EXIT_MEDIA,
            id,
            message: message.into(),
        }
    }
}

pub fn report_failure(tool: &str, failure: Failure) -> u8 {
    eprintln!("{tool}: error[{}]: {}", failure.id, failure.message);
    failure.exit
}

/// Host-file device used by inspection commands.
///
/// `File::open` is the important part of this type: the kernel never grants
/// this process a writable descriptor. The trait's mutation methods are
/// present only to satisfy the shared block-device interface and fail closed.
pub struct ReadOnlyImage {
    file: File,
    block_size: usize,
    total_blocks: u64,
}

impl ReadOnlyImage {
    pub fn open(path: &Path) -> Result<Self, Failure> {
        let file = File::open(path).map_err(|error| {
            Failure::host_io(format!("cannot open {} read-only: {error}", path.display()))
        })?;
        let length = file
            .metadata()
            .map_err(|error| Failure::host_io(format!("cannot stat {}: {error}", path.display())))?
            .len();
        let total_blocks = length.div_ceil(DEFAULT_BLOCK_SIZE as u64).max(1);
        Ok(Self {
            file,
            block_size: DEFAULT_BLOCK_SIZE,
            total_blocks,
        })
    }

    pub fn use_declared_geometry(&mut self, ident: &Identification) {
        // Sparse image files intentionally have a short physical tail. Once
        // the immutable identification block validates, its logical geometry
        // is authoritative and missing host-file bytes read as zero.
        self.total_blocks = ident.total_blocks;
    }
}

impl BlockDevice for ReadOnlyImage {
    fn block_size(&self) -> usize {
        self.block_size
    }

    fn total_blocks(&self) -> u64 {
        self.total_blocks
    }

    fn read_block(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        if lba >= self.total_blocks {
            return Err(BlockError::OutOfBounds {
                lba,
                total_blocks: self.total_blocks,
            });
        }
        if buf.len() != self.block_size {
            return Err(BlockError::WrongBufferSize {
                expected: self.block_size,
                actual: buf.len(),
            });
        }
        let offset = lba
            .checked_mul(self.block_size as u64)
            .ok_or_else(|| BlockError::Io(std::io::Error::other("image offset overflows u64")))?;
        self.file.seek(SeekFrom::Start(offset))?;
        let mut filled = 0;
        while filled < buf.len() {
            let count = self.file.read(&mut buf[filled..])?;
            if count == 0 {
                buf[filled..].fill(0);
                break;
            }
            filled += count;
        }
        Ok(())
    }

    fn write_block(&mut self, _lba: u64, _data: &[u8]) -> Result<(), BlockError> {
        Err(BlockError::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "read-only inspection backend rejected a write",
        )))
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        Err(BlockError::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "read-only inspection backend rejected a flush",
        )))
    }
}

pub struct HeaderView {
    pub ident: Identification,
    pub selection: Selection,
}

pub fn read_header(path: &Path) -> Result<(ReadOnlyImage, HeaderView), Failure> {
    let mut device = ReadOnlyImage::open(path)?;
    let mut block = vec![0u8; DEFAULT_BLOCK_SIZE];
    device.read_block(0, &mut block).map_err(|error| {
        block_failure("E_IDENT_READ", "cannot read identification block", error)
    })?;
    let ident = Identification::decode(&block).map_err(|error| {
        Failure::media(
            "E_IDENT",
            format!("identification block is invalid: {error}"),
        )
    })?;
    device.use_declared_geometry(&ident);
    let selection = select_checkpoint(&mut device, &ident)
        .map_err(|error| core_failure("E_CHECKPOINT", "cannot select checkpoint", error))?;
    Ok((device, HeaderView { ident, selection }))
}

pub fn core_failure(id: &'static str, context: &str, error: CoreError) -> Failure {
    match error {
        CoreError::Block(BlockError::Io(error)) => Failure::host_io(format!("{context}: {error}")),
        other => Failure::media(id, format!("{context}: {other}")),
    }
}

fn block_failure(id: &'static str, context: &str, error: BlockError) -> Failure {
    match error {
        BlockError::Io(error) => Failure::host_io(format!("{context}: {error}")),
        other => Failure::media(id, format!("{context}: {other}")),
    }
}

pub fn uuid_hex(uuid: &[u8; 16]) -> String {
    hex(uuid)
}

pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}

pub fn json_string(value: &str) -> String {
    let mut output = String::from("\"");
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if (character as u32) < 0x20 => {
                use std::fmt::Write as _;
                write!(output, "\\u{:04x}", character as u32)
                    .expect("writing to String cannot fail");
            }
            character => output.push(character),
        }
    }
    output.push('"');
    output
}

pub fn name_algorithm_name(algorithm: NameKeyAlgorithm) -> &'static str {
    match algorithm {
        NameKeyAlgorithm::LegacyIdentity => "legacy-identity",
        NameKeyAlgorithm::UnicodeNfc => "unicode-nfc",
        NameKeyAlgorithm::UnicodeNfcCasefold => "unicode-nfc-casefold",
    }
}

pub fn feature_names(ident: &Identification) -> Vec<&'static str> {
    let mut names = Vec::new();
    if ident.features.compat & COMPAT_DATA_POLICY != 0 {
        names.push("org.aros.afsplus:data-policy");
    }
    if ident.features.ro_compat & RO_COMPAT_SHARED_EXTENTS != 0 {
        names.push("org.aros.afsplus:shared-extents");
    }
    if ident.features.ro_compat & RO_COMPAT_ORPHAN_DIRECTORY != 0 {
        names.push("org.aros.afsplus:orphan-directory");
    }
    if ident.features.incompat & INCOMPAT_INTENT_LOG != 0 {
        names.push("org.aros.afsplus:intent-log");
    }
    if ident.features.incompat & INCOMPAT_INTENT_LOG_DATA_UPDATES != 0 {
        names.push("org.aros.afsplus:intent-log-data-updates");
    }
    names.sort_unstable();
    names
}

pub fn json_string_list(values: &[&str]) -> String {
    let mut output = String::from("[");
    for (index, value) in values.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        output.push_str(&json_string(value));
    }
    output.push(']');
    output
}

pub fn features_json(ident: &Identification) -> String {
    format!(
        "{{\"compat\":\"{:#018x}\",\"ro_compat\":\"{:#018x}\",\"incompat\":\"{:#018x}\",\"enabled\":{}}}",
        ident.features.compat,
        ident.features.ro_compat,
        ident.features.incompat,
        json_string_list(&feature_names(ident)),
    )
}

pub fn header_json(view: &HeaderView) -> String {
    let ident = &view.ident;
    let checkpoint = &view.selection.chosen;
    let emergency_headroom = emergency_headroom_for_volume(ident.total_blocks);
    let available_blocks = checkpoint
        .free_blocks_total
        .saturating_sub(emergency_headroom);
    let other_generation = view
        .selection
        .other
        .as_ref()
        .map_or_else(|| "null".to_owned(), |other| other.generation.to_string());
    format!(
        "\"volume\":{{\"uuid\":{},\"label\":{},\"block_size\":{},\"total_blocks\":{},\"region_blocks\":{},\"log_slots\":{},\"name_key_algorithm\":{},\"case_sensitive\":{},\"unicode_version\":\"{}.{}.{}\",\"features\":{}}},\"checkpoint\":{{\"chosen_slot\":{},\"generation\":{},\"other_generation\":{},\"committed_tx_id\":{},\"next_object_id\":{},\"free_blocks\":{},\"emergency_headroom_blocks\":{emergency_headroom},\"available_blocks\":{available_blocks},\"object_map_lba\":{},\"allocation_root_lba\":{},\"reclaim_root_lba\":{},\"shared_extent_root_lba\":{},\"slot_status\":[{},{}]}}",
        json_string(&uuid_hex(&ident.uuid)),
        json_string(&checkpoint.label),
        ident.block_size(),
        ident.total_blocks,
        ident.region_size,
        ident.log_slots,
        json_string(name_algorithm_name(ident.name_key_algorithm)),
        ident.name_key_algorithm != NameKeyAlgorithm::UnicodeNfcCasefold,
        ident.unicode_version[0],
        ident.unicode_version[1],
        ident.unicode_version[2],
        features_json(ident),
        json_string(if view.selection.chosen_slot == 0 { "A" } else { "B" }),
        checkpoint.generation,
        other_generation,
        checkpoint.committed_tx_id,
        checkpoint.next_object_id,
        checkpoint.free_blocks_total,
        checkpoint.object_map_block,
        checkpoint.allocation_root_block,
        checkpoint.reclaim_root_block,
        checkpoint.shared_extent_root_block,
        json_string(&view.selection.slot_status[0]),
        json_string(&view.selection.slot_status[1]),
    )
}

#[cfg(test)]
mod tests {
    use super::json_string;

    #[test]
    fn json_strings_escape_every_control_surface() {
        assert_eq!(
            json_string("quote\" slash\\ line\nreturn\rtab\t\u{0001} é"),
            "\"quote\\\" slash\\\\ line\\nreturn\\rtab\\t\\u0001 é\""
        );
    }
}
