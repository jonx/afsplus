//! Which compatibility profiles and implementations can take a volume, read
//! from its feature masks. The masks say it themselves: a `compat` feature
//! is ignorable, an `ro_compat` feature must be understood to write, an
//! `incompat` feature to mount at all. A profile allows a feature or not
//! ([`profiles/`](../../../profiles/)); so does an implementation.

use afsplus_format::ident::{
    Identification, COMPAT_DATA_POLICY, INCOMPAT_INTENT_LOG, INCOMPAT_INTENT_LOG_DATA_UPDATES,
    INCOMPAT_PERSISTENT_SNAPSHOTS, INCOMPAT_SECURITY_DESCRIPTORS, RO_COMPAT_ORPHAN_DIRECTORY,
    RO_COMPAT_SHARED_EXTENTS,
};

use crate::common::{json_string, json_string_list};

/// One feature: its name, the mask it lives in and the profile key that
/// allows it.
struct Feature {
    name: &'static str,
    class: Class,
    bit: u64,
    key: &'static str,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Class {
    Compat,
    RoCompat,
    Incompat,
}

const FEATURES: [Feature; 7] = [
    Feature {
        name: "org.aros.afsplus:data-policy",
        class: Class::Compat,
        bit: COMPAT_DATA_POLICY,
        key: "allow_data_policy",
    },
    Feature {
        name: "org.aros.afsplus:shared-extents",
        class: Class::RoCompat,
        bit: RO_COMPAT_SHARED_EXTENTS,
        key: "allow_shared_extents",
    },
    Feature {
        name: "org.aros.afsplus:orphan-directory",
        class: Class::RoCompat,
        bit: RO_COMPAT_ORPHAN_DIRECTORY,
        key: "allow_orphan_directory",
    },
    Feature {
        name: "org.aros.afsplus:intent-log",
        class: Class::Incompat,
        bit: INCOMPAT_INTENT_LOG,
        key: "allow_intent_log",
    },
    Feature {
        name: "org.aros.afsplus:intent-log-data-updates",
        class: Class::Incompat,
        bit: INCOMPAT_INTENT_LOG_DATA_UPDATES,
        key: "allow_intent_log_data_updates",
    },
    Feature {
        name: "org.aros.afsplus:persistent-snapshots",
        class: Class::Incompat,
        bit: INCOMPAT_PERSISTENT_SNAPSHOTS,
        key: "allow_persistent_snapshots",
    },
    Feature {
        name: "org.aros.afsplus:security-descriptors",
        class: Class::Incompat,
        bit: INCOMPAT_SECURITY_DESCRIPTORS,
        key: "allow_security_descriptors",
    },
];

/// The distributed policy files, the same the formatter is bound to.
const PROFILES: [(&str, &str); 5] = [
    (
        "reader-minimal",
        include_str!("../../../profiles/reader-minimal.toml"),
    ),
    (
        "classic-rw",
        include_str!("../../../profiles/classic-rw.toml"),
    ),
    (
        "boot-safe",
        include_str!("../../../profiles/boot-safe.toml"),
    ),
    (
        "workstation",
        include_str!("../../../profiles/workstation.toml"),
    ),
    ("full", include_str!("../../../profiles/full.toml")),
];

/// The portable C reader's compiled masks (`portable/c/reader.c`,
/// `AFSPR_SUPPORTED_INCOMPAT`): it reads, so ro_compat features do not
/// stop it, and it knows the three incompat features listed there.
const PORTABLE_C_READER_INCOMPAT: u64 =
    INCOMPAT_INTENT_LOG | INCOMPAT_INTENT_LOG_DATA_UPDATES | INCOMPAT_SECURITY_DESCRIPTORS;

/// What a policy that allows some features can do with a volume.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verdict {
    /// Every feature the volume has is allowed: read and write.
    Full,
    /// A feature that must be understood to write is not allowed: read only.
    ReadOnly,
    /// A feature that must be understood to mount is not allowed.
    CannotMount,
}

impl Verdict {
    pub fn name(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::ReadOnly => "read-only",
            Self::CannotMount => "cannot-mount",
        }
    }
}

fn enabled(ident: &Identification, feature: &Feature) -> bool {
    let mask = match feature.class {
        Class::Compat => ident.features.compat,
        Class::RoCompat => ident.features.ro_compat,
        Class::Incompat => ident.features.incompat,
    };
    mask & feature.bit != 0
}

fn allows(policy: &str, key: &str) -> bool {
    policy
        .lines()
        .any(|line| line.trim() == format!("{key} = true"))
}

/// The verdict of a policy and the enabled features beyond it.
fn judge(
    ident: &Identification,
    allowed: impl Fn(&Feature) -> bool,
) -> (Verdict, Vec<&'static str>) {
    let mut verdict = Verdict::Full;
    let mut beyond = Vec::new();
    for feature in &FEATURES {
        if enabled(ident, feature) && !allowed(feature) {
            beyond.push(feature.name);
            verdict = match (verdict, feature.class) {
                (_, Class::Incompat) | (Verdict::CannotMount, _) => Verdict::CannotMount,
                (_, Class::RoCompat) | (Verdict::ReadOnly, _) => Verdict::ReadOnly,
                (Verdict::Full, Class::Compat) => Verdict::Full,
            };
        }
    }
    beyond.sort_unstable();
    (verdict, beyond)
}

pub struct Report {
    pub profiles: Vec<(&'static str, Verdict, Vec<&'static str>)>,
    pub portable_c_reader: (Verdict, Vec<&'static str>),
}

pub fn report(ident: &Identification) -> Report {
    let profiles = PROFILES
        .iter()
        .map(|(name, policy)| {
            let (verdict, beyond) = judge(ident, |feature| allows(policy, feature.key));
            (*name, verdict, beyond)
        })
        .collect();
    let (verdict, beyond) = judge(ident, |feature| match feature.class {
        Class::Compat | Class::RoCompat => true,
        Class::Incompat => PORTABLE_C_READER_INCOMPAT & feature.bit != 0,
    });
    // A reader never writes, so read-only is as good as full for it.
    let verdict = if verdict == Verdict::ReadOnly {
        Verdict::Full
    } else {
        verdict
    };
    Report {
        profiles,
        portable_c_reader: (verdict, beyond),
    }
}

pub fn report_json(ident: &Identification) -> String {
    let r = report(ident);
    let mut out = String::from("{\"profiles\":{");
    for (index, (name, verdict, beyond)) in r.profiles.iter().enumerate() {
        if index != 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "{}:{{\"verdict\":{},\"beyond\":{}}}",
            json_string(name),
            json_string(verdict.name()),
            json_string_list(beyond)
        ));
    }
    let (verdict, beyond) = &r.portable_c_reader;
    out.push_str(&format!(
        "}},\"portable_c_reader\":{{\"verdict\":{},\"beyond\":{}}}}}",
        json_string(if *verdict == Verdict::CannotMount {
            "cannot-read"
        } else {
            "reads"
        }),
        json_string_list(beyond)
    ));
    out
}

pub fn report_text(ident: &Identification) -> String {
    let r = report(ident);
    let mut lines = Vec::new();
    for (name, verdict, beyond) in &r.profiles {
        lines.push(if beyond.is_empty() {
            format!("  {name}: {}", verdict.name())
        } else {
            format!(
                "  {name}: {} (beyond it: {})",
                verdict.name(),
                beyond.join(", ")
            )
        });
    }
    let (verdict, beyond) = &r.portable_c_reader;
    lines.push(if *verdict == Verdict::CannotMount {
        format!("  portable C reader: cannot read ({})", beyond.join(", "))
    } else {
        "  portable C reader: reads".to_owned()
    });
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ident(compat: u64, ro_compat: u64, incompat: u64) -> Identification {
        Identification {
            uuid: [0; 16],
            block_shift: 12,
            checksum_algorithm: 1,
            region_size: 4096,
            log_slots: if incompat & INCOMPAT_INTENT_LOG != 0 {
                8
            } else {
                0
            },
            features: afsplus_format::ident::FeatureFlags {
                compat,
                ro_compat,
                incompat,
            },
            name_key_algorithm: afsplus_format::ident::NameKeyAlgorithm::UnicodeNfc,
            unicode_version: [16, 0, 0],
            total_blocks: 4096,
            checkpoint_slots: [1, 2],
            metadata_start: 3,
            label: String::new(),
        }
    }

    #[test]
    fn a_workstation_volume_is_read_only_for_the_classic_profiles() {
        let r = report(&ident(
            COMPAT_DATA_POLICY,
            RO_COMPAT_SHARED_EXTENTS | RO_COMPAT_ORPHAN_DIRECTORY,
            INCOMPAT_INTENT_LOG | INCOMPAT_INTENT_LOG_DATA_UPDATES,
        ));
        let by_name = |n: &str| r.profiles.iter().find(|p| p.0 == n).unwrap();
        assert_eq!(by_name("classic-rw").1, Verdict::ReadOnly);
        assert_eq!(
            by_name("classic-rw").2,
            vec![
                "org.aros.afsplus:data-policy",
                "org.aros.afsplus:shared-extents"
            ]
        );
        assert_eq!(by_name("workstation").1, Verdict::Full);
        assert_eq!(r.portable_c_reader.0, Verdict::Full);
    }

    #[test]
    fn a_snapshot_volume_cannot_be_mounted_by_a_constrained_profile_nor_read_by_the_c_reader() {
        let r = report(&ident(
            0,
            RO_COMPAT_ORPHAN_DIRECTORY,
            INCOMPAT_INTENT_LOG | INCOMPAT_PERSISTENT_SNAPSHOTS,
        ));
        let by_name = |n: &str| r.profiles.iter().find(|p| p.0 == n).unwrap();
        assert_eq!(by_name("reader-minimal").1, Verdict::CannotMount);
        assert_eq!(by_name("full").1, Verdict::Full);
        assert_eq!(r.portable_c_reader.0, Verdict::CannotMount);
        assert_eq!(
            r.portable_c_reader.1,
            vec!["org.aros.afsplus:persistent-snapshots"]
        );
    }

    /// Control: a policy that allows everything gives full everywhere.
    #[test]
    fn every_profile_key_named_here_exists_in_every_policy_file() {
        for (name, policy) in PROFILES {
            for feature in &FEATURES {
                assert!(
                    policy
                        .lines()
                        .any(|l| l.trim().starts_with(&format!("{} = ", feature.key))),
                    "{name} does not decide {}",
                    feature.key
                );
            }
        }
    }
}
