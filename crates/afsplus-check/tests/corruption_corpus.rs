//! Systematic corruption-corpus qualification for the Rust checker.

use std::collections::BTreeMap;
use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use afsplus_check::corpus::{
    build_corruption_corpus, validate_corruption_case, write_corruption_corpus,
    CORPUS_MANIFEST_VERSION,
};
use afsplus_check::REPORT_SCHEMA_VERSION;

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new(label: &str) -> Self {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("afsplus-{label}-{}-{sequence}", std::process::id()));
        assert!(
            !path.exists(),
            "temporary path collision: {}",
            path.display()
        );
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        if self.0.exists() {
            fs::remove_dir_all(&self.0).expect("remove dedicated corpus test directory");
        }
    }
}

#[test]
fn every_wire_surface_has_integrity_and_semantic_cases_without_panics() {
    let cases = build_corruption_corpus().expect("build deterministic corpus");
    assert_eq!(cases.len(), 12);

    let mut per_surface = BTreeMap::new();
    for case in &cases {
        *per_surface.entry(case.surface).or_insert(0usize) += 1;
        let outcome = catch_unwind(AssertUnwindSafe(|| validate_corruption_case(case)));
        let report = outcome
            .unwrap_or_else(|_| panic!("checker panicked for corpus case {}", case.id))
            .unwrap_or_else(|error| panic!("corpus case {} failed: {error}", case.id));
        assert_eq!(
            report.render_json(),
            validate_corruption_case(case)
                .expect("repeat corpus check")
                .render_json(),
            "checker JSON changed between identical runs for {}",
            case.id
        );
    }

    assert_eq!(
        per_surface,
        BTreeMap::from([
            ("bitmap", 2),
            ("checkpoint", 2),
            ("identification", 2),
            ("intent-log", 2),
            ("object", 2),
            ("tree", 2),
        ])
    );
}

#[test]
fn exported_sparse_corpus_and_reports_are_byte_reproducible() {
    let first = TempDirectory::new("corruption-corpus-a");
    let second = TempDirectory::new("corruption-corpus-b");
    assert_eq!(write_corruption_corpus(first.path()).unwrap(), 12);
    assert_eq!(write_corruption_corpus(second.path()).unwrap(), 12);

    let first_manifest = fs::read(first.path().join("manifest.json")).unwrap();
    let second_manifest = fs::read(second.path().join("manifest.json")).unwrap();
    assert_eq!(first_manifest, second_manifest);
    let manifest = String::from_utf8(first_manifest).unwrap();
    assert!(manifest.contains(&format!(
        "\"corpus_schema_version\":{CORPUS_MANIFEST_VERSION}"
    )));
    assert!(manifest.contains(&format!(
        "\"checker_schema_version\":{REPORT_SCHEMA_VERSION}"
    )));

    for case in build_corruption_corpus().unwrap() {
        for suffix in ["img", "report.json"] {
            let filename = format!("{}.{suffix}", case.id);
            assert_eq!(
                fs::read(first.path().join(&filename)).unwrap(),
                fs::read(second.path().join(&filename)).unwrap(),
                "generated artifact differs for {filename}"
            );
        }
    }
}

#[test]
fn exporter_never_overwrites_an_existing_destination() {
    let destination = TempDirectory::new("corruption-corpus-existing");
    fs::create_dir(destination.path()).unwrap();
    fs::write(destination.path().join("keep"), b"owned by caller").unwrap();

    let error = write_corruption_corpus(destination.path()).unwrap_err();
    assert!(error.contains("refusing to replace existing output"));
    assert_eq!(
        fs::read(destination.path().join("keep")).unwrap(),
        b"owned by caller"
    );
}
