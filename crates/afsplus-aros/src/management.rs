//! Versioned structured state of a running handler (ADR-025).
//!
//! Management tools are thin clients: they ask the mounted instance and print
//! what it returns. The document names its schema and version; a consumer
//! rejects a version it does not know. Hand-written emitter, no dependency.

use afsplus_block::BlockDevice;
use afsplus_core::MountMode;

use crate::health::{
    HEALTH_CORRUPTION, HEALTH_DEVICE_ERROR, HEALTH_INTERNAL_FAULT, HEALTH_REPLAY_PENDING,
};
use crate::{ArosAdapter, ArosError};

pub const INFO_SCHEMA: &str = "afsplus-handler-info";
pub const INFO_SCHEMA_VERSION: u32 = 1;

const HEALTH_FLAG_NAMES: [(u32, &str); 4] = [
    (HEALTH_DEVICE_ERROR, "device_error"),
    (HEALTH_CORRUPTION, "corruption"),
    (HEALTH_REPLAY_PENDING, "replay_pending"),
    (HEALTH_INTERNAL_FAULT, "internal_fault"),
];

fn json_string(text: &str) -> String {
    let mut out = String::from("\"");
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            control if (control as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", control as u32));
            }
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

fn json_names<'a>(names: impl Iterator<Item = &'a str>) -> String {
    let quoted: Vec<String> = names.map(json_string).collect();
    format!("[{}]", quoted.join(","))
}

fn mount_mode_name(mode: MountMode) -> &'static str {
    match mode {
        MountMode::ReadWrite => "read-write",
        MountMode::ReadOnly => "read-only",
        MountMode::NoChanges => "no-changes",
        MountMode::Recovery => "recovery",
    }
}

impl<D: BlockDevice> ArosAdapter<D> {
    /// One JSON object describing the volume, mount, capabilities, health and
    /// handle usage. Field order is fixed, and a reader keyed on names keeps
    /// working when a counter is added, so an addition leaves the version
    /// alone; a rename or a removal raises it.
    pub fn info_json(&mut self) -> Result<String, ArosError> {
        let identity = self.vfs.identity();
        let policy = self.volume_policy();
        let health = self.health()?;
        let uuid: String = identity
            .uuid
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let unicode = policy.statfs.unicode_version;
        Ok(format!(
            "{{\"schema\":{},\"schema_version\":{},\
             \"volume\":{{\"uuid\":{},\"label\":{},\"block_size\":{},\"total_blocks\":{},\
             \"free_blocks\":{},\"available_blocks\":{},\"max_name_bytes\":{},\
             \"case_sensitive\":{},\"unicode_version\":{},\
             \"features\":{{\"compat\":{},\"ro_compat\":{},\"incompat\":{}}}}},\
             \"mount\":{{\"mode\":{},\"generation\":{},\"pending_intent_records\":{},\
             \"pending_orphans\":{}}},\
             \"capabilities\":{},\
             \"health\":{{\"flags\":{},\"device_errors\":{},\"corruption_errors\":{},\
             \"no_space_errors\":{},\"internal_faults\":{},\
             \"checkpoint_fallbacks\":{},\"reclaim_backlog_highs\":{},\
             \"free_count_mismatches\":{},\"events_recorded\":{},\
             \"events_dropped\":{},\"last_error\":{}}},\
             \"handles\":{{\"locks\":{},\"files\":{},\"watches\":{}}}}}",
            json_string(INFO_SCHEMA),
            INFO_SCHEMA_VERSION,
            json_string(&uuid),
            json_string(&identity.label),
            policy.statfs.block_size,
            policy.statfs.total_blocks,
            policy.statfs.free_blocks,
            policy.statfs.available_blocks,
            policy.statfs.max_name_bytes,
            policy.statfs.case_sensitive,
            json_string(&format!("{}.{}.{}", unicode[0], unicode[1], unicode[2])),
            identity.compat,
            identity.ro_compat,
            identity.incompat,
            json_string(mount_mode_name(policy.mount_mode)),
            health.generation,
            health.pending_intent_records,
            health.pending_orphans,
            json_names(policy.capabilities.names()),
            json_names(
                HEALTH_FLAG_NAMES
                    .iter()
                    .filter(|(bit, _)| health.flags & bit != 0)
                    .map(|(_, name)| *name)
            ),
            health.device_errors,
            health.corruption_errors,
            health.no_space_errors,
            health.internal_faults,
            health.checkpoint_fallbacks,
            health.reclaim_backlog_highs,
            health.free_count_mismatches,
            health.events_recorded,
            health.events_dropped,
            health.last_error,
            self.locks.len(),
            self.files.len(),
            self.watches.len(),
        ))
    }
}
