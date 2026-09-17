//! A new operational entry must not silently bypass the shared diagnostic guard.
use std::collections::BTreeSet;

#[test]
fn every_mutable_operational_entry_has_a_registered_outer_guard() {
    let sources = [
        include_str!("../src/volume.rs"),
        include_str!("../src/volume/attributes.rs"),
        include_str!("../src/volume/metadata.rs"),
        include_str!("../src/volume/security.rs"),
        include_str!("../src/volume/snapshots.rs"),
    ];
    let mut guarded = BTreeSet::new();
    for source in sources {
        for definition in source.split("\n    pub fn ").skip(1) {
            let opening = definition.find('{').unwrap();
            let signature = &definition[..opening];
            if !signature.contains("&mut self") {
                continue;
            }
            let name: String = signature
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            // Recorder configuration and escaped raw-device access are not
            // filesystem operations and cannot own a filesystem call scope.
            if matches!(
                name.as_str(),
                "replace_flight_recorder" | "flight_recorder_mut" | "device_mut"
            ) {
                continue;
            }
            let body = definition[opening + 1..].trim_start();
            assert!(
                body.starts_with("self.trace_api(")
                    || body.starts_with("self.trace_api_infallible("),
                "unguarded API: {name}"
            );
            let variant: String = name
                .split('_')
                .map(|word| {
                    let mut letters = word.chars();
                    letters.next().unwrap().to_uppercase().collect::<String>() + letters.as_str()
                })
                .collect();
            let method = body.split("crate::flight::ApiMethod::").nth(1).unwrap();
            assert!(
                method.starts_with(&(variant.clone() + ",")),
                "wrong API identity: {name}"
            );
            assert!(guarded.insert(variant), "duplicate API: {name}");
        }
    }
    let flight = include_str!("../src/flight.rs");
    let registry = flight
        .split("pub enum ApiMethod {")
        .nth(1)
        .unwrap()
        .split('}')
        .next()
        .unwrap();
    let registered: BTreeSet<_> = registry
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(name, _)| name.trim().to_owned())
        .collect();
    assert_eq!(guarded, registered, "API registration/coverage mismatch");
}
