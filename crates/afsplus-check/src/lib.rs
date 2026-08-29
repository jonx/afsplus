//! AFS+ volume checker.
//!
//! Reuses the core's reachable-state validation (ADR-015: the filesystem and
//! `afsplus-check` must not become two divergent interpretations of the
//! format) and reports on both checkpoint slots, not only the one a mount
//! would choose.
//!
//! The checker never writes (`spec/compatibility-rules.md`: repair tools are
//! stricter than normal mounts; this prototype checker is verify-only).

use afsplus_block::BlockDevice;
use afsplus_format::checkpoint::Checkpoint;
use afsplus_format::ident::Identification;
use afsplus_core::verify::validate_checkpoint_reachable;

/// Versioned structured-output schema (ADR-025).
pub const REPORT_SCHEMA_VERSION: u32 = 1;

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
    pub generation: u64,
    pub chosen_slot: usize,
    pub object_count: usize,
    pub reachable_metadata_blocks: usize,
    pub next_free_block: u64,
}

impl CheckReport {
    pub fn is_clean(&self) -> bool {
        self.errors.is_empty()
    }

    pub fn render_text(&self) -> String {
        let mut out = String::new();
        if let Some(v) = &self.volume {
            out.push_str(&format!(
                "volume {} label \"{}\" blocks {} generation {} (slot {})\n\
                 objects {} reachable metadata blocks {} next free block {}\n",
                v.uuid_hex,
                v.label,
                v.total_blocks,
                v.generation,
                if v.chosen_slot == 0 { "A" } else { "B" },
                v.object_count,
                v.reachable_metadata_blocks,
                v.next_free_block,
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
        out.push_str(if self.is_clean() { "clean\n" } else { "NOT CLEAN\n" });
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
                 \"generation\":{},\"chosen_slot\":{},\"objects\":{},\
                 \"reachable_metadata_blocks\":{},\"next_free_block\":{}}},",
                json_string(&v.uuid_hex),
                json_string(&v.label),
                v.total_blocks,
                v.generation,
                v.chosen_slot,
                v.object_count,
                v.reachable_metadata_blocks,
                v.next_free_block,
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
        report.errors.push(format!("cannot read identification block: {e}"));
        return report;
    }
    let ident = match Identification::decode(&buf) {
        Ok(ident) => ident,
        Err(e) => {
            report.errors.push(format!("identification block invalid: {e}"));
            return report;
        }
    };
    if ident.total_blocks > dev.total_blocks() {
        report.errors.push(format!(
            "identification declares {} blocks but device has {}",
            ident.total_blocks,
            dev.total_blocks()
        ));
        return report;
    }

    // Examine both slots independently.
    let mut candidates: Vec<(usize, Checkpoint)> = Vec::new();
    for (slot, lba) in ident.checkpoint_slots.iter().enumerate() {
        match dev.read_block(*lba, &mut buf) {
            Ok(()) => match Checkpoint::decode(&buf, &ident.uuid) {
                Ok(checkpoint) => {
                    report.slots.push(format!("valid, generation {}", checkpoint.generation));
                    candidates.push((slot, checkpoint));
                }
                Err(e) => report.slots.push(format!("invalid: {e}")),
            },
            Err(e) => report.slots.push(format!("unreadable: {e}")),
        }
    }
    if candidates.is_empty() {
        report.errors.push("no valid checkpoint slot".into());
        return report;
    }

    candidates.sort_by_key(|(_, c)| core::cmp::Reverse(c.generation));
    if candidates.len() == 2 && candidates[0].1.generation == candidates[1].1.generation {
        report.errors.push("both checkpoint slots carry the same generation".into());
    }

    let mut chosen = None;
    for (slot, checkpoint) in &candidates {
        match validate_checkpoint_reachable(dev, &ident, checkpoint) {
            Ok(state) => {
                chosen = Some((*slot, *checkpoint, state));
                break;
            }
            Err(e) => {
                // An invalid newest slot is exactly the state a torn commit
                // leaves behind; mount falls back, so the volume is still
                // usable — report it as a warning, not corruption.
                report.warnings.push(format!(
                    "slot {} (generation {}) has invalid reachable state: {e}",
                    if *slot == 0 { "A" } else { "B" },
                    checkpoint.generation
                ));
            }
        }
    }

    match chosen {
        Some((slot, checkpoint, state)) => {
            report.volume = Some(VolumeSummary {
                uuid_hex: hex(&ident.uuid),
                label: ident.label.clone(),
                total_blocks: ident.total_blocks,
                generation: checkpoint.generation,
                chosen_slot: slot,
                object_count: state.objects.len(),
                reachable_metadata_blocks: state.reachable_blocks.len(),
                next_free_block: checkpoint.next_free_block,
            });
        }
        None => report.errors.push("no checkpoint slot yields a valid reachable state".into()),
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
