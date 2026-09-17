//! `afsplus-explain` over a real image file: the three questions, both
//! output forms, the statuses, and the JSON read back by an independent
//! parser.
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use afsplus_block::{BlockDevice, FileBackend};
use afsplus_core::{mount, AttributeWriteMode};
use afsplus_format::{Timespec, DEFAULT_BLOCK_SIZE, OBJECT_ROOT};

struct TempDir(PathBuf);

impl TempDir {
    fn new(test: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("afsplus-explain-{test}-{}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn time(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 7,
    }
}

fn format_image(path: &Path) {
    let output = Command::new(env!("CARGO_BIN_EXE_mkafsplus"))
        .args([
            "--size-mib",
            "2",
            "--uuid",
            "00112233-4455-6677-8899-aabbccddeeff",
            "--timestamp-seconds",
            "123",
            "--profile",
            "workstation",
        ])
        .arg(path.as_os_str())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
}

fn explain(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_afsplus-explain"))
        .args(args)
        .output()
        .unwrap()
}

/// Evaluate a Python expression over the parsed JSON document `d`: the
/// parser is not ours, so a malformed document fails here.
fn query(json: &str, expression: &str) -> String {
    let mut child = Command::new("python3")
        .args([
            "-c",
            &format!("import json,sys; d=json.load(sys.stdin); print(({expression}))"),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(json.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{json}\n{output:?}");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[test]
fn the_three_questions_in_both_forms() {
    let temp = TempDir::new("questions");
    let image = temp.0.join("image.afsplus");
    format_image(&image);
    let device = FileBackend::open(&image, DEFAULT_BLOCK_SIZE, 512).unwrap();
    let mut volume = mount(device).unwrap();
    let dir = volume
        .create_directory(OBJECT_ROOT, "Drawer", time(200))
        .unwrap();
    let file = volume
        .create_file_in_directory(dir, "note \"1\"", &[7u8; 5000], time(201))
        .unwrap();
    volume
        .set_object_comment(file, "Résumé\tof 1992", time(202))
        .unwrap();
    volume
        .set_attributes(
            file,
            &[("aros.icon", Some(&[1u8; 300])), ("user.tag", Some(b"x"))],
            AttributeWriteMode::Create,
            time(203),
        )
        .unwrap();
    volume.sync().unwrap();
    drop(volume.into_device());
    let image = image.to_str().unwrap();

    // Path, machine-readable.
    let output = explain(&["--json", image, "path", "/Drawer/note \"1\""]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let json = String::from_utf8(output.stdout).unwrap();
    assert!(
        json.starts_with("{\"schema_version\":2,\"kind\":\"path\","),
        "{json}"
    );
    assert_eq!(query(&json, "d['partial'], d['problems']"), "(False, [])");
    assert_eq!(
        query(&json, "[(c['name'], c['object']) for c in d['components']]"),
        format!("[('Drawer', {dir}), ('note \"1\"', {file})]")
    );
    assert_eq!(
        query(
            &json,
            "d['object']['id'], d['object']['type'], d['object']['size_bytes'], d['object']['comment']"
        ),
        format!("({file}, 'file', 5000, 'Résumé\\tof 1992')")
    );
    assert_eq!(
        query(
            &json,
            "[(a['name'], a['value_len']) for a in d['object']['attributes']['attributes']]"
        ),
        "[('aros.icon', 300), ('user.tag', 1)]"
    );
    assert_eq!(
        query(
            &json,
            "d['object']['modified'], d['object']['changed']['seconds']"
        ),
        "({'seconds': 201, 'nanoseconds': 7}, 203)"
    );
    assert_eq!(
        query(&json, "d['object']['names'], d['object']['security']"),
        format!("([{{'parent': {dir}, 'name': 'note \"1\"'}}], None)")
    );
    let record_block: u64 = query(&json, "d['object']['record_block']").parse().unwrap();

    // The same object by ID, human-readable.
    let output = explain(&[image, "object", &file.to_string()]);
    assert_eq!(output.status.code(), Some(0));
    let text = String::from_utf8(output.stdout).unwrap();
    for line in [
        format!("object {file}: file, record at block {record_block}, 1 link(s)"),
        "  comment: Résumé\tof 1992".to_owned(),
        "    aros.icon (300 bytes)".to_owned(),
        format!("  named \"note \\\"1\\\"\" in directory {dir}"),
    ] {
        assert!(text.contains(&line), "missing {line:?} in\n{text}");
    }

    // The record block, both forms; then a free block and the first block.
    let output = explain(&["--json", image, "block", &record_block.to_string()]);
    assert_eq!(output.status.code(), Some(0));
    let json = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        query(
            &json,
            "d['kind'], d['block']['allocation'], d['block']['verdict'], d['block']['roles'], d['block']['identity']['magic'], d['block']['identity']['checksum_valid']"
        ),
        format!(
            "('block', 'allocated', 'owned', [{{'role': 'object-record', 'object': {file}}}], 'AFSO', True)"
        )
    );
    let output = explain(&[image, "block", &record_block.to_string()]);
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        text.contains(&format!("  role: object-record object={file}")),
        "{text}"
    );
    let output = explain(&["--json", image, "block", "500"]);
    let json = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        query(
            &json,
            "d['block']['verdict'], d['block']['roles'], d['block']['identity']"
        ),
        "('free', [], None)"
    );
    let output = explain(&["--json", image, "block", "0"]);
    let json = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        query(
            &json,
            "d['block']['allocation'], d['block']['roles'][0]['role']"
        ),
        "('reserved', 'identification')"
    );
}

#[test]
fn statuses_for_absent_things_bad_usage_and_a_damaged_image() {
    let temp = TempDir::new("statuses");
    let image_path = temp.0.join("image.afsplus");
    format_image(&image_path);
    let image = image_path.to_str().unwrap();

    for (args, status) in [
        (vec![image, "object", "999"], 1),
        (vec![image, "path", "/nothing"], 1),
        (vec![image, "block", "99999999"], 1),
        (vec![image, "block", "x"], 2),
        (vec![image, "inode", "1"], 2),
        (vec![image, "block"], 2),
        (vec!["--yaml", image, "block", "1"], 2),
        (vec!["/nonexistent/image", "block", "1"], 2),
    ] {
        let output = explain(&args);
        assert_eq!(output.status.code(), Some(status), "{args:?}: {output:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
        assert!(!output.stderr.is_empty(), "{args:?}");
    }
    assert_eq!(explain(&["--help"]).status.code(), Some(0));

    // Break the root directory block: the answer still comes, marked partial,
    // with the media status.
    let output = explain(&["--json", image, "object", "1"]);
    assert_eq!(output.status.code(), Some(0));
    let json = String::from_utf8(output.stdout).unwrap();
    let mut device = FileBackend::open(&image_path, DEFAULT_BLOCK_SIZE, 512).unwrap();
    let mut block = vec![0u8; DEFAULT_BLOCK_SIZE];
    let root_directory = (0..512)
        .find(|lba| {
            device.read_block(*lba, &mut block).unwrap();
            &block[0..4] == b"AFSD"
                || (&block[0..4] == b"AFST" && block[8..16] == 1u64.to_le_bytes())
        })
        .expect("root directory block");
    block[100] ^= 0xff;
    device.write_block(root_directory, &block).unwrap();
    device.flush().unwrap();
    drop(device);
    let output = explain(&["--json", image, "object", "1"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let damaged = String::from_utf8(output.stdout).unwrap();
    assert_eq!(query(&json, "d['partial']"), "False");
    assert_eq!(
        query(&damaged, "d['partial'], len(d['problems']) > 0"),
        "(True, True)"
    );
}

#[test]
fn extent_checkpoint_reclaim_space_and_feature() {
    let temp = TempDir::new("more");
    let image = temp.0.join("image.afsplus");
    format_image(&image);
    let device = FileBackend::open(&image, DEFAULT_BLOCK_SIZE, 512).unwrap();
    let mut volume = mount(device).unwrap();
    let file = volume
        .create_file_in_directory(OBJECT_ROOT, "sparse", &[7u8; 100], time(200))
        .unwrap();
    volume
        .write_file_at(file, 20 * 4096, &[9u8; 5000], time(201))
        .unwrap();
    let doomed = volume
        .create_file_in_directory(OBJECT_ROOT, "doomed", &[1u8; 9000], time(202))
        .unwrap();
    volume
        .delete_file(OBJECT_ROOT, "doomed", time(203))
        .unwrap();
    let generation = volume.checkpoint().generation;
    let free = volume.checkpoint().free_blocks_total;
    volume.sync().unwrap();
    drop(volume.into_device());
    let _ = doomed;
    let image = image.to_str().unwrap();
    let ask = |args: &[&str]| {
        let mut full = vec!["--json", image];
        full.extend_from_slice(args);
        let output = explain(&full);
        assert_eq!(output.status.code(), Some(0), "{args:?}: {output:?}");
        String::from_utf8(output.stdout).unwrap()
    };

    let id = file.to_string();
    let json = ask(&["extent", &id, "81925"]);
    assert_eq!(
        query(
            &json,
            "d['kind'], d['extent']['state'], d['extent']['logical_block'], d['extent']['offset_in_block'], d['extent']['shared'], d['extent']['unwritten']"
        ),
        "('extent', 'mapped', 20, 5, False, False)"
    );
    let physical: u64 = query(&json, "d['extent']['physical_block']")
        .parse()
        .unwrap();
    // The block the extent names is a data block of that file at that place.
    let block = ask(&["block", &physical.to_string()]);
    assert_eq!(
        query(
            &block,
            "[(r['role'], r['object'], r['logical_block']) for r in d['block']['roles']]"
        ),
        format!("[('data', {file}, 20)]")
    );
    assert_eq!(
        query(&ask(&["extent", &id, "8192"]), "d['extent']['state']"),
        "hole"
    );
    assert_eq!(
        query(&ask(&["extent", &id, "86920"]), "d['extent']['state']"),
        "beyond-end"
    );

    let json = ask(&["checkpoint"]);
    assert_eq!(
        query(
            &json,
            "d['generation'], d['checkpoint']['free_blocks_total'], d['checkpoint']['label'], sorted(s['selected'] for s in d['checkpoint']['slots']), d['checkpoint']['snapshot_roots']"
        ),
        format!("({generation}, {free}, 'AFSPlus', [False, True], None)")
    );
    assert_eq!(
        query(
            &json,
            "[s['generation'] for s in d['checkpoint']['slots'] if s['selected']]"
        ),
        format!("[{generation}]")
    );

    // The deleted file's blocks wait in the queue; one of them names its run.
    let json = ask(&["reclaim"]);
    assert_eq!(
        query(&json, "d['reclaim']['blocks'] >= 3, d['reclaim']['run']"),
        "(True, None)"
    );
    let quarantined: u64 = (0..512)
        .find(|lba| {
            let block = ask(&["block", &lba.to_string()]);
            query(
                &block,
                "any(r['role'] == 'quarantined' for r in d['block']['roles'])",
            ) == "True"
        })
        .expect("a quarantined block");
    let json = ask(&["reclaim", &quarantined.to_string()]);
    assert_eq!(
        query(
            &json,
            "d['reclaim']['block'], d['reclaim']['run']['start'] <= d['reclaim']['block'] < d['reclaim']['run']['start'] + d['reclaim']['run']['blocks']"
        ),
        format!("({quarantined}, True)")
    );
    assert_eq!(
        query(&ask(&["reclaim", "0"]), "d['reclaim']['run']"),
        "None"
    );

    let json = ask(&["space", "0"]);
    assert_eq!(
        query(
            &json,
            "d['space']['blocks'], d['space']['reserved_blocks'] + d['space']['allocated_blocks'] + d['space']['free_blocks'], d['space']['free_blocks'], d['space']['unowned_blocks']"
        ),
        format!("(512, 512, {free}, 0)")
    );

    let json = ask(&["feature"]);
    assert_eq!(
        query(&json, "len(d['features']), [f['id'] for f in d['features'] if f['class'] == 'incompat' and f['enabled']]"),
        "(7, ['org.aros.afsplus:intent-log', 'org.aros.afsplus:intent-log-data-updates'])"
    );
    let json = ask(&["feature", "org.aros.afsplus:persistent-snapshots"]);
    assert_eq!(
        query(
            &json,
            "[(f['class'], f['bit'], f['enabled']) for f in d['features']]"
        ),
        "[('incompat', 2, False)]"
    );

    // Human forms answer too, and the wrong shapes are usage errors.
    for args in [
        vec![image, "extent", &id, "0"],
        vec![image, "checkpoint"],
        vec![image, "reclaim"],
        vec![image, "space", "0"],
        vec![image, "feature"],
    ] {
        let output = explain(&args);
        assert_eq!(output.status.code(), Some(0), "{args:?}");
        assert!(!output.stdout.is_empty());
    }
    for (args, status) in [
        (vec![image, "extent", &id], 2),
        (vec![image, "checkpoint", "1"], 2),
        (vec![image, "space", "9"], 1),
        (vec![image, "extent", "1", "0"], 1),
        (vec![image, "feature", "org.aros.afsplus:nothing"], 1),
    ] {
        assert_eq!(explain(&args).status.code(), Some(status), "{args:?}");
    }
}

#[test]
fn info_and_explain_name_the_same_enabled_features() {
    let temp = TempDir::new("features");
    let image = temp.0.join("image.afsplus");
    let mut device = FileBackend::create(&image, DEFAULT_BLOCK_SIZE, 1024).unwrap();
    afsplus_core::mkfs_with_snapshots_and_security_descriptors(
        &mut device,
        &afsplus_core::MkfsParams {
            uuid: [7; 16],
            label: "Both".into(),
            region_size: 512,
            reclaim_caps: Default::default(),
            log_slots: 0,
            shared_extents: true,
            data_policy: true,
            name_policy: afsplus_core::NamePolicy::Sensitive,
            timestamp: time(1),
        },
    )
    .unwrap();
    device.flush().unwrap();
    drop(device);
    let image = image.to_str().unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_afsplus-info"))
        .args(["--json", image])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let info = query(
        &String::from_utf8(output.stdout).unwrap(),
        "d['volume']['features']['enabled']",
    );
    let output = explain(&["--json", image, "feature"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let explained = query(
        &String::from_utf8(output.stdout).unwrap(),
        "sorted(f['id'] for f in d['features'] if f['enabled'])",
    );
    assert_eq!(info, explained);
    assert_eq!(
        explained,
        "['org.aros.afsplus:data-policy', 'org.aros.afsplus:orphan-directory', 'org.aros.afsplus:persistent-snapshots', 'org.aros.afsplus:security-descriptors', 'org.aros.afsplus:shared-extents']"
    );
}
