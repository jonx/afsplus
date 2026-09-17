//! Semantic image diff against the operations that produced the images.
//!
//! Every pair below is built with the real core: format an image, commit a
//! known set of operations on a copy of it, and require the diff to be the
//! literal consequence of those operations. The diff shares no traversal
//! with the core or the checker, so each expectation is a cross-check of one
//! against the other; where the two disagree, the disagreement is the
//! finding. Three further witnesses run on every pair: the diff of an image
//! with itself is empty, the diff of the reversed pair is the mirror of the
//! diff, and the changed byte ranges are compared with a brute-force
//! comparison of the two files read through the core.
use afsplus_block::{BlockDevice, MemoryBackend};
use afsplus_check::diff::{
    diff_devices, AllocationSummary, AttributeChange, AttributeValue, ByteRange, DiffOptions,
    FieldChange, ImageDiff, LinkChange, ObjectChange, ObjectDiff, ObjectSummary, OrphanChange,
    Rename, SecurityState, SnapshotChange, TimestampField, ATTRIBUTE_VALUE_INLINE_BYTES,
    DIFF_SCHEMA_VERSION,
};
use afsplus_core::volume::{AttributeWriteMode, BatchOp, SnapshotWorkLimits};
use afsplus_core::{
    mkfs_with_options, mkfs_with_security_descriptors, mount, mount_with_snapshot_limits,
    MkfsOptions, MkfsParams, MountOptions, NamePolicy, Volume,
};
use afsplus_format::object::ObjectRecord;
use afsplus_format::{Timespec, OBJECT_ORPHAN_DIRECTORY, OBJECT_ROOT};

const BLOCK: usize = 4096;

fn time(n: i64) -> Timespec {
    Timespec {
        seconds: n,
        nanoseconds: 3,
    }
}

fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect()
}

fn formatted() -> MemoryBackend {
    let mut dev = MemoryBackend::new(BLOCK, 2048);
    mkfs_with_security_descriptors(
        &mut dev,
        &MkfsParams {
            uuid: [0x5c; 16],
            label: "Diff".into(),
            region_size: 512,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: true,
            name_policy: NamePolicy::Sensitive,
            timestamp: time(1),
        },
    )
    .unwrap();
    dev
}

fn mutate(dev: &MemoryBackend, f: impl FnOnce(&mut Volume<MemoryBackend>)) -> MemoryBackend {
    let mut volume = mount(dev.clone()).unwrap();
    f(&mut volume);
    volume.sync().unwrap();
    volume.into_device()
}

/// The base image: a file of 9000 bytes and an empty directory, committed.
struct Base {
    dev: MemoryBackend,
    seed: u64,
    dir: u64,
}

fn base() -> Base {
    let mut seed = 0;
    let mut dir = 0;
    let dev = mutate(&formatted(), |volume| {
        seed = volume
            .create_file_in_directory(OBJECT_ROOT, "seed", &pattern(9000, 1), time(2))
            .unwrap();
        dir = volume
            .create_directory(OBJECT_ROOT, "dir", time(2))
            .unwrap();
    });
    Base { dev, seed, dir }
}

fn diff_of(a: &MemoryBackend, b: &MemoryBackend) -> ImageDiff {
    let (mut x, mut y) = (a.clone(), b.clone());
    diff_devices(&mut x, &mut y, DiffOptions::default()).unwrap()
}

fn metadata_diff_of(a: &MemoryBackend, b: &MemoryBackend) -> ImageDiff {
    let (mut x, mut y) = (a.clone(), b.clone());
    diff_devices(
        &mut x,
        &mut y,
        DiffOptions {
            metadata_only: true,
        },
    )
    .unwrap()
}

/// The three changes a directory records when one of its entries changes.
fn parent_touched(
    object_id: u64,
    from: i64,
    to: i64,
    content_generation: (u64, u64),
) -> ObjectDiff {
    ObjectDiff {
        object_id,
        change: ObjectChange::Modified(vec![
            FieldChange::Timestamp {
                field: TimestampField::Modified,
                from: time(from),
                to: time(to),
            },
            FieldChange::Timestamp {
                field: TimestampField::Changed,
                from: time(from),
                to: time(to),
            },
            FieldChange::ContentGeneration {
                from: content_generation.0,
                to: content_generation.1,
            },
        ]),
    }
}

fn changed(object_id: u64, from: i64, to: i64) -> ObjectDiff {
    ObjectDiff {
        object_id,
        change: ObjectChange::Modified(vec![FieldChange::Timestamp {
            field: TimestampField::Changed,
            from: time(from),
            to: time(to),
        }]),
    }
}

fn created_file(object_id: u64, size: u64, links: u32) -> ObjectDiff {
    ObjectDiff {
        object_id,
        change: ObjectChange::Created(ObjectSummary {
            object_type: "file",
            size_bytes: size,
            link_count: links,
        }),
    }
}

fn sorted<T: std::fmt::Debug>(items: &[T]) -> Vec<String> {
    let mut out: Vec<String> = items.iter().map(|item| format!("{item:?}")).collect();
    out.sort();
    out
}

fn mirror_field(field: &FieldChange) -> FieldChange {
    match field.clone() {
        FieldChange::Type { from, to } => FieldChange::Type { from: to, to: from },
        FieldChange::Size { from, to } => FieldChange::Size { from: to, to: from },
        FieldChange::AllocatedBytes { from, to } => {
            FieldChange::AllocatedBytes { from: to, to: from }
        }
        FieldChange::LinkCount { from, to } => FieldChange::LinkCount { from: to, to: from },
        FieldChange::Protection { from, to } => FieldChange::Protection { from: to, to: from },
        FieldChange::Timestamp { field, from, to } => FieldChange::Timestamp {
            field,
            from: to,
            to: from,
        },
        FieldChange::ContentGeneration { from, to } => {
            FieldChange::ContentGeneration { from: to, to: from }
        }
        FieldChange::SymlinkTarget { from, to } => {
            FieldChange::SymlinkTarget { from: to, to: from }
        }
        FieldChange::Security {
            from,
            to,
            bytes_changed,
        } => FieldChange::Security {
            from: to,
            to: from,
            bytes_changed,
        },
        FieldChange::Comment { from, to } => FieldChange::Comment { from: to, to: from },
        FieldChange::Attributes { changes } => FieldChange::Attributes {
            changes: changes
                .into_iter()
                .map(|change| AttributeChange {
                    name: change.name,
                    from: change.to,
                    to: change.from,
                })
                .collect(),
        },
        FieldChange::Content { ranges } => FieldChange::Content { ranges },
        FieldChange::Allocation { from, to } => FieldChange::Allocation { from: to, to: from },
    }
}

/// Every image is identical to itself, and the reversed diff is this diff
/// with creation and removal, and every field, exchanged.
fn assert_identity_and_mirror(a: &MemoryBackend, b: &MemoryBackend, forward: &ImageDiff) {
    assert!(diff_of(a, a).is_empty(), "an image differs from itself");
    assert!(diff_of(b, b).is_empty(), "an image differs from itself");
    let backward = diff_of(b, a);

    let expected_objects: Vec<ObjectDiff> = forward
        .objects
        .iter()
        .map(|object| ObjectDiff {
            object_id: object.object_id,
            change: match &object.change {
                ObjectChange::Created(summary) => ObjectChange::Removed(summary.clone()),
                ObjectChange::Removed(summary) => ObjectChange::Created(summary.clone()),
                ObjectChange::Modified(fields) => {
                    ObjectChange::Modified(fields.iter().map(mirror_field).collect())
                }
            },
        })
        .collect();
    assert_eq!(sorted(&expected_objects), sorted(&backward.objects));

    let expected_links: Vec<LinkChange> = forward
        .links
        .iter()
        .map(|link| match link.clone() {
            LinkChange::Added {
                parent,
                name,
                child,
            } => LinkChange::Removed {
                parent,
                name,
                child,
            },
            LinkChange::Removed {
                parent,
                name,
                child,
            } => LinkChange::Added {
                parent,
                name,
                child,
            },
            LinkChange::Retargeted {
                parent,
                name,
                from,
                to,
            } => LinkChange::Retargeted {
                parent,
                name,
                from: to,
                to: from,
            },
        })
        .collect();
    assert_eq!(sorted(&expected_links), sorted(&backward.links));

    let expected_renames: Vec<Rename> = forward
        .renames
        .iter()
        .map(|rename| Rename {
            object_id: rename.object_id,
            from_parent: rename.to_parent,
            from_name: rename.to_name.clone(),
            to_parent: rename.from_parent,
            to_name: rename.from_name.clone(),
        })
        .collect();
    assert_eq!(sorted(&expected_renames), sorted(&backward.renames));

    let expected_orphans: Vec<OrphanChange> = forward
        .orphans
        .iter()
        .map(|orphan| OrphanChange {
            object_id: orphan.object_id,
            added: !orphan.added,
        })
        .collect();
    assert_eq!(sorted(&expected_orphans), sorted(&backward.orphans));

    let expected_snapshots: Vec<SnapshotChange> = forward
        .snapshots
        .iter()
        .map(|snapshot| match *snapshot {
            SnapshotChange::Created {
                id,
                generation,
                object_map_root: _,
            } => SnapshotChange::Removed { id, generation },
            SnapshotChange::Removed { id, generation } => SnapshotChange::Created {
                id,
                generation,
                object_map_root: 0,
            },
            other => other,
        })
        .collect();
    let seen: Vec<SnapshotChange> = backward
        .snapshots
        .iter()
        .map(|snapshot| match *snapshot {
            SnapshotChange::Created {
                id,
                generation,
                object_map_root: _,
            } => SnapshotChange::Created {
                id,
                generation,
                object_map_root: 0,
            },
            other => other,
        })
        .collect();
    assert_eq!(sorted(&expected_snapshots), sorted(&seen));

    assert_eq!(
        forward.volume.free_blocks.1 as i128 - forward.volume.free_blocks.0 as i128,
        -(backward.volume.free_blocks.1 as i128 - backward.volume.free_blocks.0 as i128)
    );
}

fn read_file(dev: &MemoryBackend, object_id: u64) -> Vec<u8> {
    mount(dev.clone()).unwrap().read_file(object_id).unwrap()
}

/// The changed byte ranges, computed by reading both files through the core
/// and comparing them byte by byte. A byte past the end of a file is absent;
/// absent differs from any present byte.
fn brute_force_ranges(a: &MemoryBackend, b: &MemoryBackend, object_id: u64) -> Vec<ByteRange> {
    let left = read_file(a, object_id);
    let right = read_file(b, object_id);
    let mut ranges = Vec::new();
    let mut open: Option<ByteRange> = None;
    for at in 0..left.len().max(right.len()) {
        if left.get(at) == right.get(at) {
            if let Some(range) = open.take() {
                ranges.push(range);
            }
        } else {
            match &mut open {
                Some(range) => range.end = at as u64 + 1,
                None => {
                    open = Some(ByteRange {
                        start: at as u64,
                        end: at as u64 + 1,
                    })
                }
            }
        }
    }
    if let Some(range) = open {
        ranges.push(range);
    }
    ranges
}

fn content_ranges(diff: &ImageDiff, object_id: u64) -> Vec<ByteRange> {
    for object in &diff.objects {
        if object.object_id != object_id {
            continue;
        }
        if let ObjectChange::Modified(fields) = &object.change {
            for field in fields {
                if let FieldChange::Content { ranges } = field {
                    return ranges.clone();
                }
            }
        }
    }
    Vec::new()
}

// ---------------------------------------------------------------------------

#[test]
fn create_reports_the_object_and_its_link() {
    let base = base();
    let mut made = 0;
    let after = mutate(&base.dev, |volume| {
        made = volume
            .create_file_in_directory(OBJECT_ROOT, "made", b"hello", time(3))
            .unwrap();
    });
    let diff = diff_of(&base.dev, &after);

    assert_eq!(diff.schema_version, DIFF_SCHEMA_VERSION);
    assert_eq!(
        diff.links,
        vec![LinkChange::Added {
            parent: OBJECT_ROOT,
            name: "made".into(),
            child: made,
        }]
    );
    assert_eq!(
        diff.objects,
        vec![
            parent_touched(OBJECT_ROOT, 2, 3, (3, 4)),
            created_file(made, 5, 1),
        ]
    );
    assert!(diff.renames.is_empty());
    assert!(diff.orphans.is_empty());
    assert!(diff.snapshots.is_empty());
    assert!(diff.problems.is_empty());
    assert_eq!(diff.volume.label, ("Diff".into(), "Diff".into()));
    assert_eq!(
        diff.volume.next_object_id.1,
        diff.volume.next_object_id.0 + 1
    );
    assert!(!diff.is_empty());
    assert_identity_and_mirror(&base.dev, &after, &diff);
}

#[test]
fn write_reports_the_changed_byte_range() {
    let base = base();
    let after = mutate(&base.dev, |volume| {
        volume
            .write_file_at(base.seed, 100, &pattern(50, 9), time(3))
            .unwrap();
    });
    let diff = diff_of(&base.dev, &after);

    assert_eq!(
        diff.objects,
        vec![ObjectDiff {
            object_id: base.seed,
            change: ObjectChange::Modified(vec![
                FieldChange::Timestamp {
                    field: TimestampField::Modified,
                    from: time(2),
                    to: time(3),
                },
                FieldChange::Timestamp {
                    field: TimestampField::Changed,
                    from: time(2),
                    to: time(3),
                },
                FieldChange::ContentGeneration { from: 2, to: 4 },
                FieldChange::Content {
                    ranges: vec![ByteRange {
                        start: 100,
                        end: 150
                    }],
                },
            ]),
        }]
    );
    assert!(diff.links.is_empty());
    assert_eq!(
        content_ranges(&diff, base.seed),
        brute_force_ranges(&base.dev, &after, base.seed)
    );
    assert_identity_and_mirror(&base.dev, &after, &diff);

    // The metadata mode reports the same object without reading content.
    let metadata = metadata_diff_of(&base.dev, &after);
    assert!(metadata.metadata_only);
    assert!(content_ranges(&metadata, base.seed).is_empty());
    assert_eq!(
        metadata.objects,
        vec![ObjectDiff {
            object_id: base.seed,
            change: ObjectChange::Modified(vec![
                FieldChange::Timestamp {
                    field: TimestampField::Modified,
                    from: time(2),
                    to: time(3),
                },
                FieldChange::Timestamp {
                    field: TimestampField::Changed,
                    from: time(2),
                    to: time(3),
                },
                FieldChange::ContentGeneration { from: 2, to: 4 },
            ]),
        }]
    );
}

#[test]
fn truncate_reports_the_lost_tail() {
    let base = base();
    let after = mutate(&base.dev, |volume| {
        volume.truncate_file(base.seed, 4000, time(3)).unwrap();
    });
    let diff = diff_of(&base.dev, &after);

    assert_eq!(
        diff.objects,
        vec![ObjectDiff {
            object_id: base.seed,
            change: ObjectChange::Modified(vec![
                FieldChange::Size {
                    from: 9000,
                    to: 4000
                },
                FieldChange::AllocatedBytes {
                    from: 12288,
                    to: 4096
                },
                FieldChange::Timestamp {
                    field: TimestampField::Modified,
                    from: time(2),
                    to: time(3),
                },
                FieldChange::Timestamp {
                    field: TimestampField::Changed,
                    from: time(2),
                    to: time(3),
                },
                FieldChange::ContentGeneration { from: 2, to: 4 },
                FieldChange::Content {
                    ranges: vec![ByteRange {
                        start: 4000,
                        end: 9000
                    }],
                },
            ]),
        }]
    );
    assert_eq!(
        content_ranges(&diff, base.seed),
        brute_force_ranges(&base.dev, &after, base.seed)
    );
    assert!(diff.volume.quarantine_blocks.1 > diff.volume.quarantine_blocks.0);
    assert_identity_and_mirror(&base.dev, &after, &diff);
}

#[test]
fn rename_across_directories_is_a_move_of_one_object() {
    let base = base();
    let after = mutate(&base.dev, |volume| {
        volume
            .rename(OBJECT_ROOT, "seed", base.dir, "moved", time(3))
            .unwrap();
    });
    let diff = diff_of(&base.dev, &after);

    assert_eq!(
        diff.renames,
        vec![Rename {
            object_id: base.seed,
            from_parent: OBJECT_ROOT,
            from_name: "seed".into(),
            to_parent: base.dir,
            to_name: "moved".into(),
        }]
    );
    assert!(diff.links.is_empty());
    assert_eq!(
        diff.objects,
        vec![
            parent_touched(OBJECT_ROOT, 2, 3, (3, 4)),
            changed(base.seed, 2, 3),
            parent_touched(base.dir, 2, 3, (3, 4)),
        ]
    );
    assert_identity_and_mirror(&base.dev, &after, &diff);
}

#[test]
fn hard_link_added_then_one_name_unlinked() {
    let base = base();
    let linked = mutate(&base.dev, |volume| {
        volume
            .link_file(base.seed, base.dir, "second", time(3))
            .unwrap();
    });
    let first = diff_of(&base.dev, &linked);
    assert_eq!(
        first.links,
        vec![LinkChange::Added {
            parent: base.dir,
            name: "second".into(),
            child: base.seed,
        }]
    );
    assert_eq!(
        first.objects,
        vec![
            ObjectDiff {
                object_id: base.seed,
                change: ObjectChange::Modified(vec![
                    FieldChange::LinkCount { from: 1, to: 2 },
                    FieldChange::Timestamp {
                        field: TimestampField::Changed,
                        from: time(2),
                        to: time(3),
                    },
                ]),
            },
            parent_touched(base.dir, 2, 3, (3, 4)),
        ]
    );
    assert!(first.renames.is_empty());
    assert_identity_and_mirror(&base.dev, &linked, &first);

    let unlinked = mutate(&linked, |volume| {
        volume.delete_file(OBJECT_ROOT, "seed", time(4)).unwrap();
    });
    let second = diff_of(&linked, &unlinked);
    assert_eq!(
        second.links,
        vec![LinkChange::Removed {
            parent: OBJECT_ROOT,
            name: "seed".into(),
            child: base.seed,
        }]
    );
    assert_eq!(
        second.objects,
        vec![
            parent_touched(OBJECT_ROOT, 2, 4, (3, 5)),
            ObjectDiff {
                object_id: base.seed,
                change: ObjectChange::Modified(vec![
                    FieldChange::LinkCount { from: 2, to: 1 },
                    FieldChange::Timestamp {
                        field: TimestampField::Changed,
                        from: time(3),
                        to: time(4),
                    },
                ]),
            },
        ]
    );
    assert_identity_and_mirror(&linked, &unlinked, &second);

    // End to end the object kept one name under another parent, which is
    // what the two states say, whatever the sequence between them.
    let whole = diff_of(&base.dev, &unlinked);
    assert_eq!(
        whole.renames,
        vec![Rename {
            object_id: base.seed,
            from_parent: OBJECT_ROOT,
            from_name: "seed".into(),
            to_parent: base.dir,
            to_name: "second".into(),
        }]
    );
    assert!(whole.links.is_empty());
}

#[test]
fn symlink_creation_and_retarget() {
    let base = base();
    let mut link = 0;
    let after = mutate(&base.dev, |volume| {
        link = volume
            .create_symlink(OBJECT_ROOT, "link", "seed", time(3))
            .unwrap();
    });
    let diff = diff_of(&base.dev, &after);
    assert_eq!(
        diff.links,
        vec![LinkChange::Added {
            parent: OBJECT_ROOT,
            name: "link".into(),
            child: link,
        }]
    );
    assert_eq!(
        diff.objects,
        vec![
            parent_touched(OBJECT_ROOT, 2, 3, (3, 4)),
            ObjectDiff {
                object_id: link,
                change: ObjectChange::Created(ObjectSummary {
                    object_type: "symlink",
                    size_bytes: 4,
                    link_count: 1,
                }),
            },
        ]
    );
    assert_identity_and_mirror(&base.dev, &after, &diff);

    // Replacing the symlink with one of another target under the same name
    // reports the target of the object that kept the name.
    let retargeted = mutate(&after, |volume| {
        volume.unlink_symlink(OBJECT_ROOT, "link", time(4)).unwrap();
        volume
            .create_symlink(OBJECT_ROOT, "link", "dir", time(4))
            .unwrap();
    });
    let second = diff_of(&after, &retargeted);
    let targets: Vec<&FieldChange> = second
        .objects
        .iter()
        .filter_map(|object| match &object.change {
            ObjectChange::Modified(fields) => fields
                .iter()
                .find(|field| matches!(field, FieldChange::SymlinkTarget { .. })),
            _ => None,
        })
        .collect();
    assert!(
        targets.is_empty(),
        "a replaced symlink is another object, not a retarget: {targets:?}"
    );
    assert!(second
        .links
        .iter()
        .any(|link| matches!(link, LinkChange::Retargeted { name, .. } if name == "link")));
}

#[test]
fn clone_file_reports_shared_allocation_without_content_change() {
    let base = base();
    let mut copy = 0;
    let after = mutate(&base.dev, |volume| {
        copy = volume
            .clone_file(base.seed, OBJECT_ROOT, "copy", time(3))
            .unwrap();
    });
    let diff = diff_of(&base.dev, &after);

    assert_eq!(
        diff.links,
        vec![LinkChange::Added {
            parent: OBJECT_ROOT,
            name: "copy".into(),
            child: copy,
        }]
    );
    assert_eq!(
        diff.objects,
        vec![
            parent_touched(OBJECT_ROOT, 2, 3, (3, 4)),
            ObjectDiff {
                object_id: base.seed,
                change: ObjectChange::Modified(vec![FieldChange::Allocation {
                    from: AllocationSummary {
                        mapped_blocks: 3,
                        shared_blocks: 0,
                        unwritten_blocks: 0,
                    },
                    to: AllocationSummary {
                        mapped_blocks: 3,
                        shared_blocks: 3,
                        unwritten_blocks: 0,
                    },
                }]),
            },
            created_file(copy, 9000, 1),
        ]
    );
    assert!(content_ranges(&diff, base.seed).is_empty());
    assert_eq!(
        brute_force_ranges(&base.dev, &after, base.seed),
        Vec::<ByteRange>::new()
    );
    assert_identity_and_mirror(&base.dev, &after, &diff);
}

#[test]
fn clone_range_reports_content_and_shared_extents() {
    let base = base();
    let mut target = 0;
    let with_target = mutate(&base.dev, |volume| {
        target = volume
            .create_file_in_directory(OBJECT_ROOT, "target", &pattern(9000, 5), time(3))
            .unwrap();
    });
    let after = mutate(&with_target, |volume| {
        volume
            .clone_range(base.seed, 0, target, 0, 8192, time(4))
            .unwrap();
    });
    let diff = diff_of(&with_target, &after);

    assert!(diff.links.is_empty());
    assert_eq!(
        diff.objects,
        vec![
            ObjectDiff {
                object_id: base.seed,
                change: ObjectChange::Modified(vec![FieldChange::Allocation {
                    from: AllocationSummary {
                        mapped_blocks: 3,
                        shared_blocks: 0,
                        unwritten_blocks: 0,
                    },
                    to: AllocationSummary {
                        mapped_blocks: 3,
                        shared_blocks: 2,
                        unwritten_blocks: 0,
                    },
                }]),
            },
            ObjectDiff {
                object_id: target,
                change: ObjectChange::Modified(vec![
                    FieldChange::Timestamp {
                        field: TimestampField::Modified,
                        from: time(3),
                        to: time(4),
                    },
                    FieldChange::Timestamp {
                        field: TimestampField::Changed,
                        from: time(3),
                        to: time(4),
                    },
                    FieldChange::ContentGeneration { from: 4, to: 5 },
                    FieldChange::Content {
                        ranges: vec![ByteRange {
                            start: 0,
                            end: 8192
                        }],
                    },
                ]),
            },
        ]
    );
    assert_eq!(
        content_ranges(&diff, target),
        brute_force_ranges(&with_target, &after, target)
    );
    assert_identity_and_mirror(&with_target, &after, &diff);
}

#[test]
fn preallocation_reports_unwritten_blocks_and_no_content_change() {
    let base = base();
    let after = mutate(&base.dev, |volume| {
        volume
            .preallocate_file(base.seed, 0, 40960, time(3))
            .unwrap();
    });
    let diff = diff_of(&base.dev, &after);

    assert_eq!(
        diff.objects,
        vec![ObjectDiff {
            object_id: base.seed,
            change: ObjectChange::Modified(vec![
                FieldChange::AllocatedBytes {
                    from: 12288,
                    to: 40960,
                },
                FieldChange::Timestamp {
                    field: TimestampField::Changed,
                    from: time(2),
                    to: time(3),
                },
                FieldChange::Allocation {
                    from: AllocationSummary {
                        mapped_blocks: 3,
                        shared_blocks: 0,
                        unwritten_blocks: 0,
                    },
                    to: AllocationSummary {
                        mapped_blocks: 10,
                        shared_blocks: 0,
                        unwritten_blocks: 7,
                    },
                },
            ]),
        }]
    );
    assert_eq!(
        brute_force_ranges(&base.dev, &after, base.seed),
        Vec::<ByteRange>::new()
    );
    assert_identity_and_mirror(&base.dev, &after, &diff);
}

#[test]
fn protection_change_is_reported_with_its_change_time() {
    let base = base();
    let after = mutate(&base.dev, |volume| {
        volume
            .set_object_protection(base.seed, 0x0000_00f5, time(3))
            .unwrap();
    });
    let diff = diff_of(&base.dev, &after);
    assert_eq!(
        diff.objects,
        vec![ObjectDiff {
            object_id: base.seed,
            change: ObjectChange::Modified(vec![
                FieldChange::Protection {
                    from: 0,
                    to: 0x0000_00f5,
                },
                FieldChange::Timestamp {
                    field: TimestampField::Changed,
                    from: time(2),
                    to: time(3),
                },
            ]),
        }]
    );
    assert_identity_and_mirror(&base.dev, &after, &diff);
}

#[test]
fn security_descriptor_set_then_replaced() {
    let base = base();
    let secured = mutate(&base.dev, |volume| {
        volume
            .set_security_descriptor(base.seed, 0x7fff_0001, 1, &pattern(200, 7), time(3))
            .unwrap();
    });
    let first = diff_of(&base.dev, &secured);
    assert_eq!(
        first.objects,
        vec![ObjectDiff {
            object_id: base.seed,
            change: ObjectChange::Modified(vec![
                FieldChange::Timestamp {
                    field: TimestampField::Changed,
                    from: time(2),
                    to: time(3),
                },
                FieldChange::Security {
                    from: None,
                    to: Some(SecurityState {
                        format: 0x7fff_0001,
                        version: 1,
                        total_len: 200,
                        diverged: false,
                    }),
                    bytes_changed: false,
                },
            ]),
        }]
    );
    assert_identity_and_mirror(&base.dev, &secured, &first);

    // A replacement of the same length and format is still a replacement:
    // the bytes are compared.
    let replaced = mutate(&secured, |volume| {
        volume
            .set_security_descriptor(base.seed, 0x7fff_0001, 1, &pattern(200, 8), time(4))
            .unwrap();
    });
    let second = diff_of(&secured, &replaced);
    let state = Some(SecurityState {
        format: 0x7fff_0001,
        version: 1,
        total_len: 200,
        diverged: false,
    });
    assert_eq!(
        second.objects,
        vec![ObjectDiff {
            object_id: base.seed,
            change: ObjectChange::Modified(vec![
                FieldChange::Timestamp {
                    field: TimestampField::Changed,
                    from: time(3),
                    to: time(4),
                },
                FieldChange::Security {
                    from: state,
                    to: state,
                    bytes_changed: true,
                },
            ]),
        }]
    );
    assert_identity_and_mirror(&secured, &replaced, &second);
}

#[test]
fn relabelling_changes_the_volume_and_nothing_else() {
    let base = base();
    let after = mutate(&base.dev, |volume| {
        volume.set_volume_label("Renamed").unwrap();
    });
    let diff = diff_of(&base.dev, &after);
    assert_eq!(diff.volume.label, ("Diff".into(), "Renamed".into()));
    assert!(diff.objects.is_empty());
    assert!(diff.links.is_empty());
    assert!(diff.problems.is_empty());
    assert!(!diff.is_empty());
    assert_identity_and_mirror(&base.dev, &after, &diff);
}

#[test]
fn a_deferred_window_reports_every_operation_it_committed() {
    let base = base();
    let mut made = 0;
    let after = mutate(&base.dev, |volume| {
        made = volume
            .window_op(
                &BatchOp::CreateFile {
                    parent_id: OBJECT_ROOT,
                    name: "w1",
                    content: b"one",
                },
                time(3),
            )
            .unwrap()
            .unwrap();
        volume
            .window_op(
                &BatchOp::Rename {
                    source_parent_id: OBJECT_ROOT,
                    source_name: "seed",
                    target_parent_id: base.dir,
                    target_name: "seed",
                    replace: false,
                },
                time(3),
            )
            .unwrap();
        volume.window_commit(time(4)).unwrap();
    });
    let diff = diff_of(&base.dev, &after);

    assert_eq!(
        diff.renames,
        vec![Rename {
            object_id: base.seed,
            from_parent: OBJECT_ROOT,
            from_name: "seed".into(),
            to_parent: base.dir,
            to_name: "seed".into(),
        }]
    );
    assert_eq!(
        diff.links,
        vec![LinkChange::Added {
            parent: OBJECT_ROOT,
            name: "w1".into(),
            child: made,
        }]
    );
    assert_eq!(
        diff.objects,
        vec![
            parent_touched(OBJECT_ROOT, 2, 3, (3, 4)),
            changed(base.seed, 2, 3),
            parent_touched(base.dir, 2, 3, (3, 4)),
            created_file(made, 3, 1),
        ]
    );
    // One checkpoint published the whole window.
    assert_eq!(diff.volume.generation.1, diff.volume.generation.0 + 1);
    assert_identity_and_mirror(&base.dev, &after, &diff);
}

#[test]
fn unlinking_an_open_object_reports_the_orphan() {
    let base = base();
    let after = mutate(&base.dev, |volume| {
        assert_eq!(
            volume.orphan_file(OBJECT_ROOT, "seed", time(3)).unwrap(),
            base.seed
        );
    });
    let diff = diff_of(&base.dev, &after);

    assert_eq!(
        diff.orphans,
        vec![OrphanChange {
            object_id: base.seed,
            added: true,
        }]
    );
    assert_eq!(
        diff.links,
        vec![LinkChange::Removed {
            parent: OBJECT_ROOT,
            name: "seed".into(),
            child: base.seed,
        }]
    );
    assert_eq!(
        diff.objects,
        vec![
            parent_touched(OBJECT_ROOT, 2, 3, (3, 5)),
            ObjectDiff {
                object_id: OBJECT_ORPHAN_DIRECTORY,
                change: ObjectChange::Created(ObjectSummary {
                    object_type: "directory",
                    size_bytes: 0,
                    link_count: 1,
                }),
            },
            changed(base.seed, 2, 3),
        ]
    );
    assert_identity_and_mirror(&base.dev, &after, &diff);
}

#[test]
fn snapshot_creation_is_reported_by_the_registry() {
    let mut dev = MemoryBackend::new(BLOCK, 2048);
    mkfs_with_options(
        &mut dev,
        &MkfsParams {
            uuid: [0x5d; 16],
            label: "Snap".into(),
            region_size: 512,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: true,
            name_policy: NamePolicy::Sensitive,
            timestamp: time(1),
        },
        MkfsOptions {
            persistent_snapshots: true,
        },
    )
    .unwrap();
    let open = |device: MemoryBackend| {
        mount_with_snapshot_limits(
            device,
            MountOptions::default(),
            SnapshotWorkLimits {
                max_edit_records: 4096,
                max_views: 128,
                reclaim_records: 8,
            },
        )
        .unwrap()
    };
    let before = {
        let mut volume = open(dev);
        volume
            .create_file_in_directory(OBJECT_ROOT, "seed", b"x", time(2))
            .unwrap();
        volume.sync().unwrap();
        volume.into_device()
    };
    let (id, after) = {
        let mut volume = open(before.clone());
        let id = volume.snapshot_create(time(3)).unwrap();
        volume.sync().unwrap();
        (id, volume.into_device())
    };
    let diff = diff_of(&before, &after);

    assert_eq!(diff.snapshots.len(), 1, "{:?}", diff.snapshots);
    match diff.snapshots[0] {
        SnapshotChange::Created {
            id: reported,
            generation,
            object_map_root,
        } => {
            assert_eq!(reported, id);
            assert_eq!(generation, diff.volume.generation.0);
            assert_ne!(object_map_root, 0);
        }
        other => panic!("expected a created snapshot, got {other:?}"),
    }
    assert_eq!(
        diff.volume.snapshot_next_id,
        (id, id + 1),
        "the registry control record advances"
    );
    assert!(diff.objects.is_empty());
    assert!(diff.problems.is_empty());
    assert_identity_and_mirror(&before, &after, &diff);
}

// --- negative controls ------------------------------------------------------

#[test]
fn negative_control_a_corrupted_expectation_does_not_match() {
    let base = base();
    let after = mutate(&base.dev, |volume| {
        volume
            .write_file_at(base.seed, 100, &pattern(50, 9), time(3))
            .unwrap();
    });
    let diff = diff_of(&base.dev, &after);

    let correct = vec![ByteRange {
        start: 100,
        end: 150,
    }];
    assert_eq!(content_ranges(&diff, base.seed), correct);

    // The same expectation with one end moved by a single byte, and with the
    // wrong object: a check that cannot fail proves nothing.
    let one_byte_wrong = vec![ByteRange {
        start: 100,
        end: 151,
    }];
    assert_ne!(content_ranges(&diff, base.seed), one_byte_wrong);
    assert_ne!(
        content_ranges(&diff, base.seed),
        Vec::<ByteRange>::new(),
        "the file did change"
    );
    assert_ne!(
        diff.objects,
        vec![created_file(base.seed, 9000, 1)],
        "a write is not a creation"
    );
    assert!(diff_of(&base.dev, &base.dev).is_empty());
    assert!(!diff_of(&base.dev, &after).is_empty());
}

/// The live object record of `object_id`: the copy with the highest
/// generation, found by scanning the image with the record codec.
fn live_record(dev: &MemoryBackend, object_id: u64) -> (u64, ObjectRecord, u64) {
    let mut found: Option<(u64, ObjectRecord, u64)> = None;
    let mut buf = vec![0u8; BLOCK];
    let mut device = dev.clone();
    for lba in 0..device.total_blocks() {
        if device.read_block(lba, &mut buf).is_err() {
            continue;
        }
        if let Ok((record, generation)) = ObjectRecord::decode_metadata_with_generation(&buf) {
            if record.object_id != object_id {
                continue;
            }
            if found.as_ref().is_none_or(|(_, _, seen)| generation > *seen) {
                found = Some((lba, record, generation));
            }
        }
    }
    found.expect("the object has a record")
}

#[test]
fn negative_control_a_record_resealed_under_another_size_is_reported() {
    let base = base();
    let (lba, record, generation) = live_record(&base.dev, base.seed);
    assert_eq!(record.size_bytes, 9000);
    let mut tampered = base.dev.clone();
    let mut forged = record;
    forged.size_bytes = 9007;
    tampered.apply_raw(lba, &forged.encode(BLOCK, generation).unwrap());

    let diff = diff_of(&base.dev, &tampered);
    assert_eq!(
        diff.objects,
        vec![ObjectDiff {
            object_id: base.seed,
            change: ObjectChange::Modified(vec![
                FieldChange::Size {
                    from: 9000,
                    to: 9007,
                },
                FieldChange::Content {
                    ranges: vec![ByteRange {
                        start: 9000,
                        end: 9007,
                    }],
                },
            ]),
        }],
        "a resealed record is a valid block and a changed object"
    );
    assert!(diff.problems.is_empty());
}

// --- damaged images ---------------------------------------------------------

#[test]
fn a_damaged_object_record_is_reported_and_nothing_is_concluded_from_it() {
    let base = base();
    let (lba, _, _) = live_record(&base.dev, base.seed);
    let mut damaged = base.dev.clone();
    damaged.apply_raw(lba, &vec![0u8; BLOCK]);

    let diff = diff_of(&base.dev, &damaged);
    assert!(diff.has_problems(), "{diff:?}");
    assert!(
        diff.problems
            .iter()
            .any(|problem| problem.contains(&format!("object {}", base.seed))),
        "{:?}",
        diff.problems
    );
    assert!(
        !diff
            .objects
            .iter()
            .any(|object| object.object_id == base.seed),
        "an unreadable record is uncompared, never removed: {:?}",
        diff.objects
    );
    assert!(!diff.is_empty());
    assert!(!diff.render_json().is_empty());
}

#[test]
fn a_damaged_object_map_reports_a_problem_instead_of_removing_every_object() {
    let base = base();
    let root = mount(base.dev.clone())
        .unwrap()
        .checkpoint()
        .object_map_block;
    let mut damaged = base.dev.clone();
    damaged.apply_raw(root, &vec![0u8; BLOCK]);

    let diff = diff_of(&base.dev, &damaged);
    assert!(diff.has_problems(), "{diff:?}");
    assert!(
        diff.objects.is_empty(),
        "a damaged object map removes nothing: {:?}",
        diff.objects
    );
    assert!(diff.links.is_empty());
    assert!(diff
        .problems
        .iter()
        .any(|problem| problem.contains("object map")));
}

#[test]
fn a_damaged_directory_reports_a_problem_instead_of_removing_entries() {
    let base = base();
    let directory_root = mount(base.dev.clone())
        .unwrap()
        .stat(OBJECT_ROOT)
        .unwrap()
        .unwrap()
        .data_root;
    let mut damaged = base.dev.clone();
    damaged.apply_raw(directory_root, &vec![0u8; BLOCK]);

    let diff = diff_of(&base.dev, &damaged);
    assert!(diff.has_problems(), "{diff:?}");
    assert!(
        diff.links.is_empty(),
        "a damaged directory removes no link: {:?}",
        diff.links
    );
    assert!(diff.renames.is_empty());
}

#[test]
fn an_image_without_a_committed_state_is_an_error_not_a_panic() {
    let base = base();
    let mut destroyed = base.dev.clone();
    destroyed.apply_raw(1, &vec![0u8; BLOCK]);
    destroyed.apply_raw(2, &vec![0u8; BLOCK]);
    let (mut x, mut y) = (base.dev.clone(), destroyed);
    let error = diff_devices(&mut x, &mut y, DiffOptions::default()).unwrap_err();
    assert!(error.contains("checkpoint"), "{error}");

    let mut nothing = MemoryBackend::new(BLOCK, 64);
    let mut left = base.dev.clone();
    let error = diff_devices(&mut left, &mut nothing, DiffOptions::default()).unwrap_err();
    assert!(error.contains("identification"), "{error}");
}

// --- stored comments (ADR-106) ---------------------------------------------

fn comment_change(object_id: u64, from: &str, to: &str, at: (i64, i64)) -> ObjectDiff {
    ObjectDiff {
        object_id,
        change: ObjectChange::Modified(vec![
            FieldChange::Timestamp {
                field: TimestampField::Changed,
                from: time(at.0),
                to: time(at.1),
            },
            FieldChange::Comment {
                from: from.into(),
                to: to.into(),
            },
        ]),
    }
}

#[test]
fn a_comment_set_then_changed_then_cleared() {
    let base = base();
    let commented = mutate(&base.dev, |volume| {
        volume
            .set_object_comment(base.seed, "first note", time(3))
            .unwrap();
    });
    let first = diff_of(&base.dev, &commented);
    assert_eq!(
        first.objects,
        vec![comment_change(base.seed, "", "first note", (2, 3))]
    );
    assert!(first.links.is_empty());
    assert!(first.problems.is_empty());
    assert_identity_and_mirror(&base.dev, &commented, &first);

    let rewritten = mutate(&commented, |volume| {
        volume
            .set_object_comment(base.seed, "second note", time(4))
            .unwrap();
    });
    let second = diff_of(&commented, &rewritten);
    assert_eq!(
        second.objects,
        vec![comment_change(
            base.seed,
            "first note",
            "second note",
            (3, 4)
        )]
    );
    assert_identity_and_mirror(&commented, &rewritten, &second);

    let cleared = mutate(&rewritten, |volume| {
        volume.set_object_comment(base.seed, "", time(5)).unwrap();
    });
    let third = diff_of(&rewritten, &cleared);
    assert_eq!(
        third.objects,
        vec![comment_change(base.seed, "second note", "", (4, 5))]
    );
    assert_identity_and_mirror(&rewritten, &cleared, &third);

    // The record holds what it held before the comment existed, so only the
    // change time separates the two images.
    let round_trip = diff_of(&base.dev, &cleared);
    assert_eq!(
        round_trip.objects,
        vec![ObjectDiff {
            object_id: base.seed,
            change: ObjectChange::Modified(vec![FieldChange::Timestamp {
                field: TimestampField::Changed,
                from: time(2),
                to: time(5),
            }]),
        }]
    );

    // The comment is metadata: it is reported without reading any content.
    let metadata = metadata_diff_of(&base.dev, &commented);
    assert_eq!(
        metadata.objects,
        vec![comment_change(base.seed, "", "first note", (2, 3))]
    );
    assert_eq!(metadata.schema_version, 3);
    assert!(metadata
        .render_json()
        .contains("{\"field\":\"comment\",\"from\":\"\",\"to\":\"first note\"}"));
    assert!(first
        .render_human()
        .contains("comment added \"first note\""));
}

#[test]
fn clone_file_carries_the_comment_and_clone_range_does_not() {
    let base = base();
    let commented = mutate(&base.dev, |volume| {
        volume
            .set_object_comment(base.seed, "carried", time(3))
            .unwrap();
    });

    // CloneFile copies the comment onto the new object.
    let mut copy = 0;
    let cloned = mutate(&commented, |volume| {
        copy = volume
            .clone_file(base.seed, OBJECT_ROOT, "copy", time(4))
            .unwrap();
    });
    assert_eq!(
        mount(cloned.clone()).unwrap().object_comment(copy).unwrap(),
        "carried"
    );
    let diff = diff_of(&commented, &cloned);
    assert!(
        !diff
            .objects
            .iter()
            .any(|object| object.object_id == copy
                && matches!(object.change, ObjectChange::Modified(_))),
        "the clone is a created object, not a modified one: {:?}",
        diff.objects
    );
    assert_identity_and_mirror(&commented, &cloned, &diff);

    // CloneRange moves data only: the destination keeps its own comment.
    let mut target = 0;
    let with_target = mutate(&commented, |volume| {
        target = volume
            .create_file_in_directory(OBJECT_ROOT, "target", &pattern(9000, 5), time(4))
            .unwrap();
        volume
            .set_object_comment(target, "destination note", time(4))
            .unwrap();
    });
    let after = mutate(&with_target, |volume| {
        volume
            .clone_range(base.seed, 0, target, 0, 8192, time(5))
            .unwrap();
    });
    assert_eq!(
        mount(after.clone())
            .unwrap()
            .object_comment(target)
            .unwrap(),
        "destination note"
    );
    let diff = diff_of(&with_target, &after);
    let comments: Vec<&FieldChange> = diff
        .objects
        .iter()
        .filter_map(|object| match &object.change {
            ObjectChange::Modified(fields) => fields
                .iter()
                .find(|field| matches!(field, FieldChange::Comment { .. })),
            _ => None,
        })
        .collect();
    assert!(
        comments.is_empty(),
        "clone_range changes no comment: {comments:?}"
    );
    assert_eq!(
        content_ranges(&diff, target),
        brute_force_ranges(&with_target, &after, target)
    );
    assert_identity_and_mirror(&with_target, &after, &diff);
}

#[test]
fn negative_control_a_corrupted_comment_expectation_does_not_match() {
    let base = base();
    let commented = mutate(&base.dev, |volume| {
        volume
            .set_object_comment(base.seed, "first note", time(3))
            .unwrap();
    });
    let diff = diff_of(&base.dev, &commented);
    assert_eq!(
        diff.objects,
        vec![comment_change(base.seed, "", "first note", (2, 3))]
    );
    // One byte of the comment, and the direction of the change: both fail.
    assert_ne!(
        diff.objects,
        vec![comment_change(base.seed, "", "first notes", (2, 3))]
    );
    assert_ne!(
        diff.objects,
        vec![comment_change(base.seed, "first note", "", (2, 3))]
    );
    assert!(diff_of(&commented, &commented).is_empty());
}

// --- extended attributes (ADR-108) ------------------------------------------

fn value(bytes: &[u8]) -> AttributeValue {
    AttributeValue {
        len: bytes.len(),
        bytes: Some(bytes.to_vec()),
        digest: None,
    }
}

fn attributes_field(object_id: u64, at: (i64, i64), changes: Vec<AttributeChange>) -> ObjectDiff {
    ObjectDiff {
        object_id,
        change: ObjectChange::Modified(vec![
            FieldChange::Timestamp {
                field: TimestampField::Changed,
                from: time(at.0),
                to: time(at.1),
            },
            FieldChange::Attributes { changes },
        ]),
    }
}

fn added(name: &str, bytes: &[u8]) -> AttributeChange {
    AttributeChange {
        name: name.into(),
        from: None,
        to: Some(value(bytes)),
    }
}

fn removed(name: &str, bytes: &[u8]) -> AttributeChange {
    AttributeChange {
        name: name.into(),
        from: Some(value(bytes)),
        to: None,
    }
}

fn replaced(name: &str, from: &[u8], to: &[u8]) -> AttributeChange {
    AttributeChange {
        name: name.into(),
        from: Some(value(from)),
        to: Some(value(to)),
    }
}

/// Every value the diff reports is what the core returns for that name on
/// the side it was reported from.
fn assert_values_agree_with_the_core(a: &MemoryBackend, b: &MemoryBackend, diff: &ImageDiff) {
    let mut left = mount(a.clone()).unwrap();
    let mut right = mount(b.clone()).unwrap();
    for object in &diff.objects {
        let ObjectChange::Modified(fields) = &object.change else {
            continue;
        };
        for field in fields {
            let FieldChange::Attributes { changes } = field else {
                continue;
            };
            for change in changes {
                let from = left.attribute(object.object_id, &change.name).unwrap();
                let to = right.attribute(object.object_id, &change.name).unwrap();
                assert_eq!(
                    change.from.as_ref().and_then(|value| value.bytes.clone()),
                    from,
                    "{}: reported from-value disagrees with the core",
                    change.name
                );
                assert_eq!(
                    change.to.as_ref().and_then(|value| value.bytes.clone()),
                    to,
                    "{}: reported to-value disagrees with the core",
                    change.name
                );
            }
        }
    }
}

fn set_attribute(dev: &MemoryBackend, id: u64, name: &str, bytes: &[u8], at: i64) -> MemoryBackend {
    mutate(dev, |volume| {
        volume
            .set_attributes(
                id,
                &[(name, Some(bytes))],
                AttributeWriteMode::Upsert,
                time(at),
            )
            .unwrap();
    })
}

#[test]
fn attributes_added_replaced_and_removed_one_by_one() {
    let base = base();

    // One attribute.
    let one = set_attribute(&base.dev, base.seed, "user.one", b"alpha", 3);
    let diff = diff_of(&base.dev, &one);
    assert_eq!(
        diff.objects,
        vec![attributes_field(
            base.seed,
            (2, 3),
            vec![added("user.one", b"alpha")]
        )]
    );
    assert_eq!(diff.schema_version, 3);
    assert!(diff.problems.is_empty());
    assert_values_agree_with_the_core(&base.dev, &one, &diff);
    assert_identity_and_mirror(&base.dev, &one, &diff);

    // Three more in one commit, reported in name order whatever the order of
    // the batch.
    let several = mutate(&one, |volume| {
        volume
            .set_attributes(
                base.seed,
                &[
                    ("user.zulu", Some(b"z".as_slice())),
                    ("aros.comment", Some(b"cc".as_slice())),
                    ("system.two", Some(b"22".as_slice())),
                ],
                AttributeWriteMode::Create,
                time(4),
            )
            .unwrap();
    });
    let diff = diff_of(&one, &several);
    assert_eq!(
        diff.objects,
        vec![attributes_field(
            base.seed,
            (3, 4),
            vec![
                added("aros.comment", b"cc"),
                added("system.two", b"22"),
                added("user.zulu", b"z"),
            ]
        )]
    );
    assert_values_agree_with_the_core(&one, &several, &diff);
    assert_identity_and_mirror(&one, &several, &diff);

    // A value replaced.
    let replaced_value = mutate(&several, |volume| {
        volume
            .set_attributes(
                base.seed,
                &[("user.one", Some(b"omega!".as_slice()))],
                AttributeWriteMode::Replace,
                time(5),
            )
            .unwrap();
    });
    let diff = diff_of(&several, &replaced_value);
    assert_eq!(
        diff.objects,
        vec![attributes_field(
            base.seed,
            (4, 5),
            vec![replaced("user.one", b"alpha", b"omega!")]
        )]
    );
    assert_values_agree_with_the_core(&several, &replaced_value, &diff);
    assert_identity_and_mirror(&several, &replaced_value, &diff);

    // One removed.
    let fewer = mutate(&replaced_value, |volume| {
        volume
            .set_attributes(
                base.seed,
                &[("system.two", None)],
                AttributeWriteMode::Upsert,
                time(6),
            )
            .unwrap();
    });
    let diff = diff_of(&replaced_value, &fewer);
    assert_eq!(
        diff.objects,
        vec![attributes_field(
            base.seed,
            (5, 6),
            vec![removed("system.two", b"22")]
        )]
    );
    assert_values_agree_with_the_core(&replaced_value, &fewer, &diff);
    assert_identity_and_mirror(&replaced_value, &fewer, &diff);

    // The last three removed: the set disappears and the record keeps no
    // reference to a chain.
    let empty = mutate(&fewer, |volume| {
        volume
            .set_attributes(
                base.seed,
                &[
                    ("user.one", None),
                    ("user.zulu", None),
                    ("aros.comment", None),
                ],
                AttributeWriteMode::Upsert,
                time(7),
            )
            .unwrap();
    });
    assert!(mount(empty.clone())
        .unwrap()
        .stat(base.seed)
        .unwrap()
        .unwrap()
        .attributes
        .is_none());
    let diff = diff_of(&fewer, &empty);
    assert_eq!(
        diff.objects,
        vec![attributes_field(
            base.seed,
            (6, 7),
            vec![
                removed("aros.comment", b"cc"),
                removed("user.one", b"omega!"),
                removed("user.zulu", b"z"),
            ]
        )]
    );
    assert_values_agree_with_the_core(&fewer, &empty, &diff);
    assert_identity_and_mirror(&fewer, &empty, &diff);

    // The set is metadata: the metadata mode reports it and reads no content.
    let metadata = metadata_diff_of(&base.dev, &one);
    assert_eq!(
        metadata.objects,
        vec![attributes_field(
            base.seed,
            (2, 3),
            vec![added("user.one", b"alpha")]
        )]
    );
    assert!(diff_of(&base.dev, &one)
        .render_json()
        .contains("{\"field\":\"attributes\",\"changes\":[{\"name\":\"user.one\",\"from\":null,\"to\":{\"length\":5,\"hex\":\"616c706861\"}}]}"));
    assert!(diff_of(&base.dev, &one)
        .render_human()
        .contains("attribute \"user.one\" added (5 bytes)"));
}

#[test]
fn a_long_attribute_value_is_reported_by_its_digest() {
    use sha2::{Digest, Sha256};
    let base = base();
    let long = pattern(ATTRIBUTE_VALUE_INLINE_BYTES + 1, 4);
    let short = pattern(ATTRIBUTE_VALUE_INLINE_BYTES, 4);

    let with_short = set_attribute(&base.dev, base.seed, "user.blob", &short, 3);
    let diff = diff_of(&base.dev, &with_short);
    assert_eq!(
        diff.objects,
        vec![attributes_field(
            base.seed,
            (2, 3),
            vec![added("user.blob", &short)]
        )],
        "a value at the bound is reported byte for byte"
    );

    let with_long = set_attribute(&base.dev, base.seed, "user.blob", &long, 3);
    let diff = diff_of(&base.dev, &with_long);
    assert_eq!(
        diff.objects,
        vec![attributes_field(
            base.seed,
            (2, 3),
            vec![AttributeChange {
                name: "user.blob".into(),
                from: None,
                to: Some(AttributeValue {
                    len: ATTRIBUTE_VALUE_INLINE_BYTES + 1,
                    bytes: None,
                    digest: Some(
                        Sha256::digest(&long)
                            .iter()
                            .map(|byte| format!("{byte:02x}"))
                            .collect::<String>()
                    ),
                }),
            }]
        )],
        "one byte past the bound is reported by its digest"
    );
    assert!(diff.render_json().contains("\"sha256\":\""));
    assert_identity_and_mirror(&base.dev, &with_long, &diff);
}

#[test]
fn clone_file_carries_the_attribute_set_and_clone_range_does_not() {
    let base = base();
    let source = set_attribute(&base.dev, base.seed, "user.carried", b"value", 3);

    let mut copy = 0;
    let cloned = mutate(&source, |volume| {
        copy = volume
            .clone_file(base.seed, OBJECT_ROOT, "copy", time(4))
            .unwrap();
    });
    assert_eq!(
        mount(cloned.clone())
            .unwrap()
            .attribute(copy, "user.carried")
            .unwrap(),
        Some(b"value".to_vec())
    );
    let diff = diff_of(&source, &cloned);
    assert!(
        !diff
            .objects
            .iter()
            .any(|object| object.object_id == copy
                && matches!(object.change, ObjectChange::Modified(_))),
        "the clone is a created object: {:?}",
        diff.objects
    );
    assert_identity_and_mirror(&source, &cloned, &diff);

    // CloneRange moves data: the destination keeps its own set.
    let mut target = 0;
    let with_target = mutate(&source, |volume| {
        target = volume
            .create_file_in_directory(OBJECT_ROOT, "target", &pattern(9000, 5), time(4))
            .unwrap();
        volume
            .set_attributes(
                target,
                &[("user.destination", Some(b"kept".as_slice()))],
                AttributeWriteMode::Create,
                time(4),
            )
            .unwrap();
    });
    let after = mutate(&with_target, |volume| {
        volume
            .clone_range(base.seed, 0, target, 0, 8192, time(5))
            .unwrap();
    });
    assert_eq!(
        mount(after.clone())
            .unwrap()
            .attribute(target, "user.destination")
            .unwrap(),
        Some(b"kept".to_vec())
    );
    assert_eq!(
        mount(after.clone())
            .unwrap()
            .attribute(target, "user.carried")
            .unwrap(),
        None
    );
    let diff = diff_of(&with_target, &after);
    let attribute_fields: Vec<&FieldChange> = diff
        .objects
        .iter()
        .filter_map(|object| match &object.change {
            ObjectChange::Modified(fields) => fields
                .iter()
                .find(|field| matches!(field, FieldChange::Attributes { .. })),
            _ => None,
        })
        .collect();
    assert!(
        attribute_fields.is_empty(),
        "clone_range changes no attribute: {attribute_fields:?}"
    );
    assert_identity_and_mirror(&with_target, &after, &diff);
}

#[test]
fn a_descriptor_a_comment_and_a_set_on_one_object() {
    let base = base();
    let descriptor = pattern(200, 7);
    let after = mutate(&base.dev, |volume| {
        volume
            .set_security_descriptor(base.seed, 0x7fff_0001, 1, &descriptor, time(3))
            .unwrap();
        volume
            .set_object_comment(base.seed, "all three", time(3))
            .unwrap();
        volume
            .set_attributes(
                base.seed,
                &[("user.three", Some(b"yes".as_slice()))],
                AttributeWriteMode::Create,
                time(3),
            )
            .unwrap();
    });
    let diff = diff_of(&base.dev, &after);
    assert_eq!(
        diff.objects,
        vec![ObjectDiff {
            object_id: base.seed,
            change: ObjectChange::Modified(vec![
                FieldChange::Timestamp {
                    field: TimestampField::Changed,
                    from: time(2),
                    to: time(3),
                },
                FieldChange::Comment {
                    from: "".into(),
                    to: "all three".into(),
                },
                FieldChange::Security {
                    from: None,
                    to: Some(SecurityState {
                        format: 0x7fff_0001,
                        version: 1,
                        total_len: 200,
                        diverged: false,
                    }),
                    bytes_changed: false,
                },
                FieldChange::Attributes {
                    changes: vec![added("user.three", b"yes")],
                },
            ]),
        }]
    );
    assert!(diff.problems.is_empty());
    assert_values_agree_with_the_core(&base.dev, &after, &diff);
    assert_identity_and_mirror(&base.dev, &after, &diff);
}

#[test]
fn a_damaged_attribute_chain_is_reported_and_no_attribute_is_concluded() {
    let base = base();
    let with_set = set_attribute(&base.dev, base.seed, "user.one", b"alpha", 3);
    let first_block = mount(with_set.clone())
        .unwrap()
        .stat(base.seed)
        .unwrap()
        .unwrap()
        .attributes
        .unwrap()
        .first_block;
    let mut damaged = with_set.clone();
    damaged.apply_raw(first_block, &vec![0u8; BLOCK]);

    let diff = diff_of(&with_set, &damaged);
    assert!(diff.has_problems(), "{diff:?}");
    assert!(
        diff.problems
            .iter()
            .any(|problem| problem.contains("attribute segment 0")),
        "{:?}",
        diff.problems
    );
    let attribute_fields: Vec<&FieldChange> = diff
        .objects
        .iter()
        .filter_map(|object| match &object.change {
            ObjectChange::Modified(fields) => fields
                .iter()
                .find(|field| matches!(field, FieldChange::Attributes { .. })),
            _ => None,
        })
        .collect();
    assert!(
        attribute_fields.is_empty(),
        "a damaged chain removes no attribute: {attribute_fields:?}"
    );
    assert!(!diff.render_json().is_empty());
}

#[test]
fn negative_control_a_corrupted_attribute_expectation_does_not_match() {
    let base = base();
    let one = set_attribute(&base.dev, base.seed, "user.one", b"alpha", 3);
    let diff = diff_of(&base.dev, &one);
    assert_eq!(
        diff.objects,
        vec![attributes_field(
            base.seed,
            (2, 3),
            vec![added("user.one", b"alpha")]
        )]
    );
    // One byte of the value, one letter of the name, and the direction.
    assert_ne!(
        diff.objects,
        vec![attributes_field(
            base.seed,
            (2, 3),
            vec![added("user.one", b"alphb")]
        )]
    );
    assert_ne!(
        diff.objects,
        vec![attributes_field(
            base.seed,
            (2, 3),
            vec![added("user.One", b"alpha")]
        )]
    );
    assert_ne!(
        diff.objects,
        vec![attributes_field(
            base.seed,
            (2, 3),
            vec![removed("user.one", b"alpha")]
        )]
    );
    assert!(diff_of(&one, &one).is_empty());
}
