//! AFS+ volume checker.
//!
//! Uses the same checkpoint selection and state loading as normal mount
//! (ADR-015: the filesystem and `afsplus-check` must not become two
//! divergent interpretations of the format), then runs the full invariant
//! sweep that normal mount does not: link counts, orphaned objects, bitmap
//! versus reachability, retired-block quarantine.
//!
//! Selection semantics deliberately match mount: the newest structurally
//! valid checkpoint is the volume's state. If it references corrupt
//! metadata, that is an error — the checker does not paper over it by
//! falling back to the older slot. The older slot gets shadow verification:
//! its findings are warnings, because mount never reads it while the newest
//! is intact.
//!
//! The checker never writes (`spec/compatibility-rules.md`: repair tools are
//! stricter than normal mounts; this prototype checker is verify-only).

pub mod crash_replay;
pub mod diff;
pub mod explain;
pub mod replay_trace;
pub mod scenario;

use afsplus_block::BlockDevice;
use afsplus_core::mount::select_checkpoint;
use afsplus_core::verify::{full_sweep, load_committed_state};
use afsplus_core::CoreError;
use afsplus_format::ident::{Identification, NameKeyAlgorithm, RO_COMPAT_SHARED_EXTENTS};

pub mod corpus;

/// Versioned structured-output schema (ADR-025).
pub const REPORT_SCHEMA_VERSION: u32 = 5;

#[derive(Debug, Default)]
pub struct CheckReport {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    /// One status line per checkpoint slot.
    pub slots: Vec<String>,
    pub volume: Option<VolumeSummary>,
}

#[derive(Debug)]
pub struct VolumeSummary {
    pub uuid_hex: String,
    pub label: String,
    pub total_blocks: u64,
    pub region_size: u32,
    pub name_key_algorithm: &'static str,
    pub case_sensitive: bool,
    pub unicode_version: String,
    pub generation: u64,
    pub chosen_slot: usize,
    pub object_count: usize,
    pub reachable_metadata_blocks: usize,
    pub reachable_data_blocks: usize,
    pub reclaim_pending_blocks: u64,
    pub reclaim_runs: usize,
    /// Valid intent-log records awaiting replay (ADR-037).
    pub log_records_pending: usize,
    pub free_blocks: u64,
}

impl CheckReport {
    pub fn is_clean(&self) -> bool {
        self.errors.is_empty()
    }

    pub fn render_text(&self) -> String {
        let mut out = String::new();
        if let Some(v) = &self.volume {
            out.push_str(&format!(
                "volume {} label \"{}\" blocks {} region size {} names {} Unicode {} generation {} (slot {})\n\
                 objects {} metadata blocks {} data blocks {} pending reclaim {} ({} runs) \
                 pending log records {} free {}\n",
                v.uuid_hex,
                v.label,
                v.total_blocks,
                v.region_size,
                v.name_key_algorithm,
                v.unicode_version,
                v.generation,
                if v.chosen_slot == 0 { "A" } else { "B" },
                v.object_count,
                v.reachable_metadata_blocks,
                v.reachable_data_blocks,
                v.reclaim_pending_blocks,
                v.reclaim_runs,
                v.log_records_pending,
                v.free_blocks,
            ));
        }
        for (i, s) in self.slots.iter().enumerate() {
            out.push_str(&format!("slot {}: {s}\n", if i == 0 { "A" } else { "B" }));
        }
        for w in &self.warnings {
            out.push_str(&format!("warning: {w}\n"));
        }
        for e in &self.errors {
            out.push_str(&format!("error: {e}\n"));
        }
        out.push_str(if self.is_clean() {
            "clean\n"
        } else {
            "NOT CLEAN\n"
        });
        out
    }

    /// Versioned machine-readable output (ADR-025). Hand-rolled emitter to
    /// keep the prototype dependency-free.
    pub fn render_json(&self) -> String {
        let mut out = String::from("{");
        out.push_str(&format!("\"schema_version\":{REPORT_SCHEMA_VERSION},"));
        out.push_str(&format!("\"clean\":{},", self.is_clean()));
        if let Some(v) = &self.volume {
            out.push_str(&format!(
                "\"volume\":{{\"uuid\":{},\"label\":{},\"total_blocks\":{},\
                 \"region_size\":{},\"name_key_algorithm\":{},\"case_sensitive\":{},\
                 \"unicode_version\":{},\"generation\":{},\"chosen_slot\":{},\"objects\":{},\
                 \"metadata_blocks\":{},\"data_blocks\":{},\"reclaim_pending_blocks\":{},\
                 \"reclaim_runs\":{},\"log_records_pending\":{},\"free_blocks\":{}}},",
                json_string(&v.uuid_hex),
                json_string(&v.label),
                v.total_blocks,
                v.region_size,
                json_string(v.name_key_algorithm),
                v.case_sensitive,
                json_string(&v.unicode_version),
                v.generation,
                v.chosen_slot,
                v.object_count,
                v.reachable_metadata_blocks,
                v.reachable_data_blocks,
                v.reclaim_pending_blocks,
                v.reclaim_runs,
                v.log_records_pending,
                v.free_blocks,
            ));
        } else {
            out.push_str("\"volume\":null,");
        }
        out.push_str(&format!(
            "\"slots\":{},\"warnings\":{},\"errors\":{}}}",
            json_string_array(&self.slots),
            json_string_array(&self.warnings),
            json_string_array(&self.errors),
        ));
        out
    }
}

/// Verifies a volume image. Read-only.
pub fn check_device<D: BlockDevice>(dev: &mut D) -> CheckReport {
    let mut report = CheckReport::default();
    let block_size = dev.block_size();
    let mut buf = vec![0u8; block_size];

    if let Err(e) = dev.read_block(0, &mut buf) {
        report
            .errors
            .push(format!("cannot read identification block: {e}"));
        return report;
    }
    let ident = match Identification::decode(&buf) {
        Ok(ident) => ident,
        Err(e) => {
            report
                .errors
                .push(format!("identification block invalid: {e}"));
            return report;
        }
    };
    let unknown_incompat = ident.features.incompat
        & !(afsplus_core::mount::SUPPORTED_INCOMPAT_FEATURES
            | afsplus_format::ident::INCOMPAT_PERSISTENT_SNAPSHOTS);
    if unknown_incompat != 0 {
        report.errors.push(format!(
            "unsupported incompatible filesystem features: {unknown_incompat:#018x}"
        ));
        return report;
    }
    let unknown_ro_compat =
        ident.features.ro_compat & !afsplus_core::mount::SUPPORTED_RO_COMPAT_FEATURES;
    if unknown_ro_compat != 0 {
        report.warnings.push(format!(
            "unknown read-only-compatible filesystem features: {unknown_ro_compat:#018x}"
        ));
    }
    if ident.total_blocks > dev.total_blocks() {
        report.errors.push(format!(
            "identification declares {} blocks but device has {}",
            ident.total_blocks,
            dev.total_blocks()
        ));
        return report;
    }
    let geo = ident.geometry();

    // Same selection as mount: newest structurally valid, ambiguity is fatal.
    let selection = match select_checkpoint(dev, &ident) {
        Ok(selection) => selection,
        Err(CoreError::AmbiguousCheckpoints(generation)) => {
            report.errors.push(format!(
                "both checkpoint slots carry generation {generation}; volume is ambiguous"
            ));
            return report;
        }
        Err(e) => {
            report.errors.push(e.to_string());
            return report;
        }
    };
    report.slots = selection.slot_status.to_vec();

    // The enabled-but-unused state is legal, but a root without its feature
    // would make shared ownership invisible to a writer. Match mount's
    // structural fail-closed contract before attempting an exhaustive walk.
    if selection.chosen.shared_extent_root_block != 0
        && ident.features.ro_compat & RO_COMPAT_SHARED_EXTENTS == 0
    {
        report
            .errors
            .push("shared-extent root present without the shared-extents feature".into());
        return report;
    }

    // The chosen checkpoint must load and pass the full sweep — errors.
    match load_committed_state(dev, &ident, &selection.chosen) {
        Ok(state) => {
            for finding in full_sweep(&state, &geo, &selection.chosen) {
                report.errors.push(finding);
            }
            // Intent-log validation (ADR-037): the valid prefix must
            // reference only blocks the committed state considers FREE; a
            // broken tail is a normal crash artifact.
            let mut log_records_pending = 0;
            match afsplus_core::intent_log::scan(
                dev,
                &geo,
                ident.log_slots,
                &ident.uuid,
                selection.chosen.generation,
                ident.features.incompat & afsplus_format::ident::INCOMPAT_INTENT_LOG_DATA_UPDATES
                    != 0,
            ) {
                Ok(scanned) => {
                    log_records_pending = scanned.records.len();
                    if let Some(note) = scanned.tail_note {
                        report.warnings.push(format!("intent log tail: {note}"));
                    }
                    for record in &scanned.records {
                        for op in &record.ops {
                            for (start, blocks) in op.data_extents() {
                                for lba in *start..*start + *blocks as u64 {
                                    if state.bitmaps.is_allocated(lba) {
                                        report.errors.push(format!(
                                            "log record {} references allocated block {lba}",
                                            record.sequence
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => report.errors.push(format!("intent log unreadable: {e}")),
            }
            report.volume = Some(VolumeSummary {
                uuid_hex: hex(&ident.uuid),
                label: selection.chosen.label.clone(),
                total_blocks: ident.total_blocks,
                region_size: ident.region_size,
                name_key_algorithm: match ident.name_key_algorithm {
                    NameKeyAlgorithm::LegacyIdentity => "legacy-identity",
                    NameKeyAlgorithm::UnicodeNfc => "unicode-nfc",
                    NameKeyAlgorithm::UnicodeNfcCasefold => "unicode-nfc-casefold",
                },
                case_sensitive: ident.name_key_algorithm != NameKeyAlgorithm::UnicodeNfcCasefold,
                unicode_version: format!(
                    "{}.{}.{}",
                    ident.unicode_version[0], ident.unicode_version[1], ident.unicode_version[2]
                ),
                generation: selection.chosen.generation,
                chosen_slot: selection.chosen_slot,
                object_count: state.objects.len(),
                reachable_metadata_blocks: state.metadata_blocks.len(),
                reachable_data_blocks: state.data_blocks.len(),
                reclaim_pending_blocks: state.reclaim_pending_blocks,
                reclaim_runs: state.reclaim_runs.len(),
                log_records_pending,
                free_blocks: state.bitmaps.free_blocks_total(),
            });
        }
        Err(e) => {
            report.errors.push(format!(
                "chosen checkpoint generation {} references invalid state: {e}",
                selection.chosen.generation
            ));
        }
    }

    // Shadow verification of the retained older checkpoint — warnings only.
    if let Some(other) = &selection.other {
        match load_committed_state(dev, &ident, other) {
            Ok(state) => {
                for finding in full_sweep(&state, &geo, other) {
                    report.warnings.push(format!(
                        "retained checkpoint generation {}: {finding}",
                        other.generation
                    ));
                }
            }
            Err(e) => report.warnings.push(format!(
                "retained checkpoint generation {} references invalid state: {e}",
                other.generation
            )),
        }
    }

    report
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn json_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn json_string_array(items: &[String]) -> String {
    let mut out = String::from("[");
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&json_string(item));
    }
    out.push(']');
    out
}
