//! The registry lists what exists (ADR-116): every identity in
//! `spec/feature-registry.toml` names a feature bit this code assigns, and
//! every bit this code assigns has an identity. Without this test the two
//! drift the moment nobody is looking, and a second implementer gates a
//! field on a bit that does not exist.
use afsplus_format::ident::{
    COMPAT_DATA_POLICY, INCOMPAT_INTENT_LOG, INCOMPAT_INTENT_LOG_DATA_UPDATES,
    INCOMPAT_PERSISTENT_SNAPSHOTS, INCOMPAT_SECURITY_DESCRIPTORS, RO_COMPAT_ORPHAN_DIRECTORY,
    RO_COMPAT_SHARED_EXTENTS,
};

const REGISTRY: &str = include_str!("../../../spec/feature-registry.toml");

/// Every feature bit this implementation assigns, with the class its word
/// implies and the identity the registry must carry.
const ASSIGNED: [(&str, &str, u64); 7] = [
    (
        "org.aros.afsplus:intent-log",
        "incompat",
        INCOMPAT_INTENT_LOG,
    ),
    (
        "org.aros.afsplus:intent-log-data-updates",
        "incompat",
        INCOMPAT_INTENT_LOG_DATA_UPDATES,
    ),
    (
        "org.aros.afsplus:persistent-snapshots",
        "incompat",
        INCOMPAT_PERSISTENT_SNAPSHOTS,
    ),
    (
        "org.aros.afsplus:security-descriptors",
        "incompat",
        INCOMPAT_SECURITY_DESCRIPTORS,
    ),
    (
        "org.aros.afsplus:shared-extents",
        "ro_compat",
        RO_COMPAT_SHARED_EXTENTS,
    ),
    (
        "org.aros.afsplus:orphan-directory",
        "ro_compat",
        RO_COMPAT_ORPHAN_DIRECTORY,
    ),
    ("org.aros.afsplus:data-policy", "compat", COMPAT_DATA_POLICY),
];

/// The registry's `[[feature]]` blocks as (id, class), read without a TOML
/// crate: the file is a flat list of `key = "value"` lines by construction.
fn registered() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for block in REGISTRY.split("[[feature]]").skip(1) {
        let field = |name: &str| {
            block
                .lines()
                .find_map(|line| line.strip_prefix(&format!("{name} = ")))
                .map(|value| value.trim().trim_matches('"').to_owned())
                .unwrap_or_else(|| panic!("a registry entry has no {name}"))
        };
        out.push((field("id"), field("class")));
    }
    out
}

#[test]
fn every_registered_identity_names_a_bit_this_code_assigns() {
    let registered = registered();
    assert!(!registered.is_empty(), "the registry parsed as empty");
    for (id, class) in &registered {
        let assigned = ASSIGNED
            .iter()
            .find(|(known, _, _)| known == id)
            .unwrap_or_else(|| {
                panic!(
                    "the registry lists {id}, which no feature bit of this code assigns: \
                     a registry identity arrives with its code (ADR-116)"
                )
            });
        assert_eq!(
            class, assigned.1,
            "{id} has class {class} in the registry and {} in the code",
            assigned.1
        );
    }
}

#[test]
fn every_bit_this_code_assigns_has_a_registered_identity() {
    let registered = registered();
    for (id, class, bit) in ASSIGNED {
        assert!(bit.count_ones() == 1, "{id} is not a single bit");
        let entry = registered
            .iter()
            .find(|(known, _)| known == id)
            .unwrap_or_else(|| panic!("this code assigns {id}, which the registry does not list"));
        assert_eq!(entry.1, class, "{id}");
    }
    assert_eq!(
        registered.len(),
        ASSIGNED.len(),
        "the registry and the assigned bits differ in length"
    );
}

#[test]
fn the_three_feature_words_assign_each_bit_once() {
    for class in ["compat", "ro_compat", "incompat"] {
        let mut seen = 0u64;
        for (id, known, bit) in ASSIGNED {
            if known != class {
                continue;
            }
            assert_eq!(seen & bit, 0, "{id} reuses a bit of the {class} word");
            seen |= bit;
        }
    }
}
