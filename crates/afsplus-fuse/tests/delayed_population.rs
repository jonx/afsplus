//! Populating a volume through a delayed mount (ADR-121), killed at any
//! instant, leaves a clean volume holding exactly what it held after some
//! earlier request: never a request in part, never a later request without
//! an earlier one, and never less than what the last sync made durable.
//!
//! The workload is what `rsync -a` and `cp -R` send through macFUSE for a
//! tree of files: a create, its mode and owner, extended attributes such as
//! `com.apple.provenance`, the data, the size announced again, a rename over
//! the final name, the times and the mode. Since metadata edits joined the
//! window, none of these commits on its own, so this also holds that a
//! window carrying them publishes them whole. Every block write is recorded;
//! the image a kill leaves is the starting image plus a prefix of them.
//!
//! The view of the volume after each request is read back through the live
//! adapter, which shows the window, so the expected states are what the
//! driver itself answered rather than a model of it.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use afsplus_block::{BlockDevice, BlockError, MemoryBackend};
use afsplus_check::check_device;
use afsplus_core::{mkfs, MkfsParams, MountOptions};
use afsplus_format::{Timespec, OBJECT_ROOT};
use afsplus_fuse::{AttributeWriteMode, FuseAdapter, FuseConfig};
use afsplus_vfs::{AccessMode, Durability, NodeKind, Vfs};

const BLOCK_SIZE: usize = 4096;

type WriteLog = Vec<(u64, Vec<u8>)>;

struct Recorder {
    inner: MemoryBackend,
    writes: Rc<RefCell<WriteLog>>,
}

impl BlockDevice for Recorder {
    fn block_size(&self) -> usize {
        self.inner.block_size()
    }
    fn total_blocks(&self) -> u64 {
        self.inner.total_blocks()
    }
    fn read_block(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        self.inner.read_block(lba, buf)
    }
    fn write_block(&mut self, lba: u64, data: &[u8]) -> Result<(), BlockError> {
        self.writes.borrow_mut().push((lba, data.to_vec()));
        self.inner.write_block(lba, data)
    }
    fn flush(&mut self) -> Result<(), BlockError> {
        self.inner.flush()
    }
}

fn formatted() -> MemoryBackend {
    let mut device = MemoryBackend::new(BLOCK_SIZE, 8192);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0xD7; 16],
            label: "Populate".into(),
            region_size: 8192,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Sensitive,
            timestamp: at(0),
        },
    )
    .unwrap();
    device
}

fn at(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn pattern(seed: u32, length: usize) -> Vec<u8> {
    let mut state = seed.wrapping_mul(2_654_435_761).wrapping_add(7);
    (0..length)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        })
        .collect()
}

/// What one path shows: kind, mode, owner, modification time, content or
/// link target, and every attribute the host lists, with its value.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Shown {
    kind: NodeKind,
    mode: u16,
    owner: (u32, u32),
    modified: Timespec,
    bytes: Vec<u8>,
    attributes: Vec<(Vec<u8>, Vec<u8>)>,
}

type State = BTreeMap<String, Option<Shown>>;

/// Resolves `path` from the root by lookups only: a listing of a directory
/// whose entries wait in the window would commit the window.
fn resolve<D: BlockDevice>(fuse: &mut FuseAdapter<D>, path: &str) -> Option<u64> {
    let mut id = OBJECT_ROOT;
    for part in path.split('/') {
        id = fuse.lookup(id, part.as_bytes()).ok()?.object_id;
    }
    Some(id)
}

fn show<D: BlockDevice>(fuse: &mut FuseAdapter<D>, path: &str) -> Option<Shown> {
    let id = resolve(fuse, path)?;
    let attributes = fuse.attributes(id).unwrap();
    let bytes = match attributes.kind {
        NodeKind::File => {
            let handle = fuse
                .open_file(id, AccessMode::ReadOnly, false, at(0))
                .unwrap();
            let bytes = fuse.read(handle, 0, attributes.size as u32 + 1).unwrap();
            fuse.close(handle).unwrap();
            bytes
        }
        NodeKind::Symlink => fuse.read_link(id).unwrap(),
        _ => Vec::new(),
    };
    let listed = fuse.list_attributes(id).unwrap();
    let attributes_shown = listed
        .split(|byte| *byte == 0)
        .filter(|name| !name.is_empty())
        .map(|name| (name.to_vec(), fuse.get_attribute(id, name).unwrap()))
        .collect();
    Some(Shown {
        kind: attributes.kind,
        mode: attributes.mode,
        owner: (attributes.uid, attributes.gid),
        // A directory's time follows its entries and is set by the commit
        // that publishes them; only a file's is compared.
        modified: if attributes.kind == NodeKind::Directory {
            Timespec::default()
        } else {
            attributes.modified
        },
        bytes,
        attributes: attributes_shown,
    })
}

fn state<D: BlockDevice>(fuse: &mut FuseAdapter<D>, paths: &[String]) -> State {
    paths
        .iter()
        .map(|path| (path.clone(), show(fuse, path)))
        .collect()
}

/// A path the expected state does not name did not exist yet.
fn same(expected: &State, shown: &State) -> bool {
    shown
        .iter()
        .all(|(path, shown)| expected.get(path).cloned().flatten() == *shown)
}

struct Run {
    base: MemoryBackend,
    log: WriteLog,
    paths: Vec<String>,
    /// The state after each request, with the number of block writes issued
    /// by then.
    states: Vec<(usize, State)>,
    /// Block-write counts at which a sync had returned, with the state index
    /// it made durable.
    synced: Vec<(usize, usize)>,
}

struct Driver {
    fuse: FuseAdapter<Recorder>,
    writes: Rc<RefCell<WriteLog>>,
    clock: i64,
    paths: Vec<String>,
    states: Vec<(usize, State)>,
}

impl Driver {
    fn tick(&mut self) -> Timespec {
        self.clock += 1;
        at(self.clock)
    }

    fn id(&mut self, path: &str) -> u64 {
        resolve(&mut self.fuse, path).unwrap()
    }

    fn known(&mut self, path: &str) {
        if !self.paths.iter().any(|known| known == path) {
            self.paths.push(path.to_owned());
        }
    }

    /// Records the state every request leaves.
    fn after_request(&mut self) {
        let paths = self.paths.clone();
        let shown = state(&mut self.fuse, &paths);
        let writes = self.writes.borrow().len();
        self.states.push((writes, shown));
    }

    /// The maintenance thread after the volume has been idle long enough to
    /// commit the window.
    fn idle(&mut self) {
        self.clock += 2;
        let now = at(self.clock);
        while self.fuse.maintenance_step(now) {}
        self.after_request();
    }

    fn mkdir(&mut self, parent: &str, name: &str) {
        let parent_id = if parent.is_empty() {
            OBJECT_ROOT
        } else {
            self.id(parent)
        };
        let now = self.tick();
        self.fuse
            .create_directory(parent_id, name.as_bytes(), Some(0o755), now)
            .unwrap();
        self.known(&join(parent, name));
        self.after_request();
    }

    fn set_mode(&mut self, path: &str, mode: u16) {
        let id = self.id(path);
        let now = self.tick();
        self.fuse.set_posix_mode(id, mode, now).unwrap();
        self.after_request();
    }

    fn set_times(&mut self, path: &str, modified: Timespec) {
        let id = self.id(path);
        let now = self.tick();
        self.fuse.set_times(id, modified, now).unwrap();
        self.after_request();
    }

    fn set_attribute(&mut self, path: &str, name: &[u8], value: &[u8]) {
        let id = self.id(path);
        let now = self.tick();
        self.fuse
            .set_attribute(id, name, value, AttributeWriteMode::Upsert, now)
            .unwrap();
        self.after_request();
    }

    /// What `rsync -a` sends for one file: a temporary name, its owner, the
    /// provenance attribute, the data, the size again, the rename, the times
    /// and the mode.
    fn rsync_file(&mut self, parent: &str, name: &str, content: &[u8], seed: u32) {
        let parent_id = self.id(parent);
        let temporary = format!(".{name}.tmp");
        let now = self.tick();
        let (attributes, handle) = self
            .fuse
            .create_file(
                parent_id,
                temporary.as_bytes(),
                AccessMode::ReadWrite,
                Some(0o600),
                now,
            )
            .unwrap();
        let id = attributes.object_id;
        self.known(&join(parent, &temporary));
        self.known(&join(parent, name));
        self.after_request();
        let now = self.tick();
        self.fuse.set_owner(id, Some(501), Some(20), now).unwrap();
        self.after_request();
        self.fuse.close(handle).unwrap();
        self.set_attribute(
            &join(parent, &temporary),
            b"com.apple.provenance",
            &seed.to_le_bytes(),
        );
        let now = self.tick();
        let handle = self
            .fuse
            .open_file(id, AccessMode::ReadWrite, false, now)
            .unwrap();
        for (index, piece) in content.chunks(24_000).enumerate() {
            let now = self.tick();
            self.fuse
                .write(handle, (index * 24_000) as u64, piece, now)
                .unwrap();
            self.after_request();
        }
        let now = self.tick();
        self.fuse
            .truncate(id, Some(handle), content.len() as u64, now)
            .unwrap();
        self.after_request();
        let now = self.tick();
        self.fuse
            .rename(
                parent_id,
                temporary.as_bytes(),
                parent_id,
                name.as_bytes(),
                true,
                now,
            )
            .unwrap();
        self.after_request();
        self.fuse.close(handle).unwrap();
        self.set_times(&join(parent, name), at(1_000 + i64::from(seed)));
        self.set_mode(&join(parent, name), 0o644);
    }

    /// What `cp -R` sends: the final name at once, the data, the times, the
    /// mode and the source's attributes.
    fn cp_file(&mut self, parent: &str, name: &str, content: &[u8], seed: u32) {
        let parent_id = self.id(parent);
        let now = self.tick();
        let (_, handle) = self
            .fuse
            .create_file(
                parent_id,
                name.as_bytes(),
                AccessMode::ReadWrite,
                Some(0o644),
                now,
            )
            .unwrap();
        let path = join(parent, name);
        self.known(&path);
        self.after_request();
        for (index, piece) in content.chunks(24_000).enumerate() {
            let now = self.tick();
            self.fuse
                .write(handle, (index * 24_000) as u64, piece, now)
                .unwrap();
            self.after_request();
        }
        self.fuse.close(handle).unwrap();
        self.set_times(&path, at(2_000 + i64::from(seed)));
        self.set_mode(&path, 0o640);
        self.set_attribute(&path, b"com.example.tag", &seed.to_be_bytes());
        // A second write of the same attribute in the same window replaces
        // the chain the window staged for the first.
        self.set_attribute(&path, b"com.example.tag", b"second");
    }

    fn unlink(&mut self, parent: &str, name: &str) {
        let parent_id = self.id(parent);
        let now = self.tick();
        self.fuse
            .unlink_file(parent_id, name.as_bytes(), now)
            .unwrap();
        self.after_request();
    }
}

fn join(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_owned()
    } else {
        format!("{parent}/{name}")
    }
}

fn run() -> Run {
    let base = formatted();
    let writes = Rc::new(RefCell::new(Vec::new()));
    let device = Recorder {
        inner: base.clone(),
        writes: Rc::clone(&writes),
    };
    let mut fuse = FuseAdapter::new(
        Vfs::mount(device, MountOptions::default()).unwrap(),
        // The macOS driver's configuration; a delayed mount ignores the
        // durable data replies.
        FuseConfig {
            durable_data_replies: true,
            empty_value_removes: true,
            ..FuseConfig::default()
        },
    );
    fuse.set_inline_maintenance(false);
    fuse.set_durability(Durability::DELAYED).unwrap();
    let mut driver = Driver {
        fuse,
        writes,
        clock: 10,
        paths: Vec::new(),
        states: Vec::new(),
    };
    let mut synced = Vec::new();
    driver.after_request();

    driver.mkdir("", "a");
    driver.mkdir("a", "b");
    driver.mkdir("", "c");
    driver.set_attribute("c", b"com.example.drawer", b"c");
    let sizes = [0usize, 100, 5_000, 20_000, 70_000];
    for index in 0..24u32 {
        let parent = ["a", "a/b", "c"][index as usize % 3];
        let name = format!("f{index}");
        let content = pattern(index, sizes[index as usize % sizes.len()]);
        if index % 2 == 0 {
            driver.rsync_file(parent, &name, &content, index);
        } else {
            driver.cp_file(parent, &name, &content, index);
        }
        // A file rewritten in place by rsync: a rename over a name whose
        // object, attributes included, is retired by the same window.
        if index % 4 == 3 {
            let content = pattern(100 + index, 3_000);
            driver.rsync_file(parent, &name, &content, 100 + index);
        }
        if index % 7 == 6 {
            let victim = format!("f{}", index - 2);
            let victim_parent = ["a", "a/b", "c"][(index as usize - 2) % 3];
            driver.unlink(victim_parent, &victim);
        }
        if index % 5 == 4 {
            driver.idle();
        }
        if index == 12 {
            // A drawer made, given an attribute, and removed again inside
            // one window leaves nothing behind.
            driver.mkdir("", "gone");
            driver.set_attribute("gone", b"com.example.gone", b"x");
            let now = driver.tick();
            driver
                .fuse
                .remove_directory(OBJECT_ROOT, b"gone", now)
                .unwrap();
            driver.after_request();
            driver.fuse.sync_filesystem().unwrap();
            let writes = driver.writes.borrow().len();
            synced.push((writes, driver.states.len() - 1));
        }
    }
    // rsync sets a drawer's times last, after its entries.
    driver.set_times("a", at(500));
    driver.set_times("a/b", at(501));
    driver.fuse.sync_filesystem().unwrap();
    let writes = driver.writes.borrow().len();
    synced.push((writes, driver.states.len() - 1));
    driver.after_request();

    let log = driver.writes.borrow().clone();
    Run {
        base,
        log,
        paths: driver.paths,
        states: driver.states,
        synced,
    }
}

#[test]
fn a_delayed_population_killed_anywhere_is_some_earlier_complete_state() {
    let run = run();
    let last = &run.states.last().unwrap().1;
    assert!(
        last.values().filter(|shown| shown.is_some()).count() > 20,
        "the workload left too little to check"
    );

    let mut image = run.base.clone();
    let mut earliest = 0usize;
    let mut problems = Vec::new();
    let mut checked = 0;
    // The states the cuts recovered: a kill must lose something somewhere,
    // or the comparison proves nothing.
    let mut reached = std::collections::BTreeSet::new();
    for cut in 0..=run.log.len() {
        if cut > 0 {
            let (lba, data) = &run.log[cut - 1];
            image.write_block(*lba, data).unwrap();
        }
        checked += 1;
        let report = check_device(&mut image.clone());
        if !report.is_clean() {
            problems.push(format!("cut {cut}: checker: {}", report.errors.join("; ")));
            continue;
        }
        let vfs = match Vfs::mount(image.clone(), MountOptions::default()) {
            Ok(vfs) => vfs,
            Err(error) => {
                problems.push(format!("cut {cut}: does not mount: {error:?}"));
                continue;
            }
        };
        let mut recovered = FuseAdapter::new(vfs, FuseConfig::default());
        let shown = state(&mut recovered, &run.paths);
        // Only states whose requests had been issued by this cut can
        // match, and none before what a returned sync made durable.
        let floor = run
            .synced
            .iter()
            .filter(|(writes, _)| *writes <= cut)
            .map(|(_, index)| *index)
            .max()
            .unwrap_or(0)
            .max(earliest);
        let matched = (floor..run.states.len())
            .take_while(|index| run.states[*index].0 <= cut)
            .find(|index| same(&run.states[*index].1, &shown));
        match matched {
            Some(index) => {
                earliest = index;
                reached.insert(index);
            }
            None => problems.push(format!(
                "cut {cut}: the volume is not the state after any request from {floor} on"
            )),
        }
    }
    assert!(checked > 200, "only {checked} cuts checked");
    assert!(
        problems.is_empty(),
        "{} of {checked} images a kill could leave are wrong:\n{}",
        problems.len(),
        problems
            .iter()
            .take(10)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
    // The last image is what the final sync made durable, and earlier cuts
    // recovered earlier states, one per commit the population made.
    assert!(earliest >= run.synced.last().unwrap().1);
    assert!(
        reached.len() >= 5,
        "the cuts recovered only the states {reached:?}"
    );
}

/// The point of the window: the whole population costs a few commits, not
/// several per file.
#[test]
fn a_delayed_population_commits_a_few_times_not_per_file() {
    let run = run();
    // Block writes per file are the proxy here; flushes are measured on
    // the mount. With each metadata edit committing on its own, as before
    // it joined the window, the same workload wrote about 60 per file.
    let writes = run.log.len();
    let files = 24 + 6;
    eprintln!("{writes} block writes for {files} files");
    assert!(
        writes / files < 25,
        "{writes} block writes for {files} files"
    );
}
