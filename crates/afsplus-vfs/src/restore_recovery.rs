//! ADR-133 explicit, scoped, generation-bound restore-stage maintenance.
use super::*;

pub const RESTORE_RECOVERY_CURSOR_MAX: usize = 24 * 1024;
const MAX_DEPTH: usize = 64;
// The deepest cursor, every frame with a maximal name, fits the transport.
const _: () = assert!(
    4 + 16 + 4 * 8 + (MAX_DEPTH + 1) * (5 * 8 + NAME_MAX_UTF8_BYTES) + 4
        <= RESTORE_RECOVERY_CURSOR_MAX
);

#[derive(Debug, Clone, Copy)]
pub struct RestoreRecoveryLimits {
    pub scan_entries: usize,
    pub work_steps: usize,
    /// Depth below the selected live scope (depth zero), including stage root.
    pub max_depth: usize,
}

#[derive(Debug, Default)]
pub struct RestoreRecoveryReport {
    pub scanned: u32,
    pub removed: u32,
    pub advanced: u32,
    pub blocked: u64,
    pub blocked_object: ObjectId,
    pub done: bool,
    pub cursor: Vec<u8>,
}

#[derive(Clone)]
struct Frame {
    parent: u64,
    object: u64,
    ordinal: u64,
    stage: u64,
    name: String,
}
struct Cursor {
    uuid: [u8; 16],
    scope: u64,
    generation: u64,
    blocked: u64,
    frames: Vec<Frame>,
}

fn take<'a>(bytes: &mut &'a [u8], count: usize) -> Result<&'a [u8], VfsError> {
    if bytes.len() < count {
        return Err(VfsError::Invalid);
    }
    let (value, rest) = bytes.split_at(count);
    *bytes = rest;
    Ok(value)
}
fn number(bytes: &mut &[u8]) -> Result<u64, VfsError> {
    Ok(u64::from_le_bytes(take(bytes, 8)?.try_into().unwrap()))
}
impl Cursor {
    fn decode(mut bytes: &[u8]) -> Result<Self, VfsError> {
        if bytes.len() < 4 || bytes.len() > RESTORE_RECOVERY_CURSOR_MAX {
            return Err(VfsError::Invalid);
        }
        let (body, sum) = bytes.split_at(bytes.len() - 4);
        if afsplus_format::crc32c::crc32c(body) != u32::from_le_bytes(sum.try_into().unwrap()) {
            return Err(VfsError::Invalid);
        }
        bytes = body;
        if take(&mut bytes, 4)? != b"RRC1" {
            return Err(VfsError::Invalid);
        }
        let uuid = take(&mut bytes, 16)?.try_into().unwrap();
        let scope = number(&mut bytes)?;
        let generation = number(&mut bytes)?;
        let blocked = number(&mut bytes)?;
        let count = number(&mut bytes)?;
        if count == 0 || count > (MAX_DEPTH + 1) as u64 {
            return Err(VfsError::Invalid);
        }
        let mut frames = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let parent = number(&mut bytes)?;
            let object = number(&mut bytes)?;
            let ordinal = number(&mut bytes)?;
            let stage = number(&mut bytes)?;
            let size = number(&mut bytes)?;
            if object == 0 || size > NAME_MAX_UTF8_BYTES as u64 {
                return Err(VfsError::Invalid);
            }
            let name = std::str::from_utf8(take(&mut bytes, size as usize)?)
                .map_err(|_| VfsError::Invalid)?
                .to_owned();
            if name.contains(['/', '\0']) || name == "." || name == ".." {
                return Err(VfsError::Invalid);
            }
            frames.push(Frame {
                parent,
                object,
                ordinal,
                stage,
                name,
            });
        }
        if !bytes.is_empty()
            || frames[0].object != scope
            || frames[0].parent != 0
            || !frames[0].name.is_empty()
            || frames[0].stage != 0
        {
            return Err(VfsError::Invalid);
        }
        for pair in frames.windows(2) {
            if pair[1].parent != pair[0].object
                || pair[1].name.is_empty()
                || frames.iter().filter(|f| f.object == pair[1].object).count() != 1
            {
                return Err(VfsError::Invalid);
            }
            if pair[0].stage != 0 && pair[1].stage != pair[0].stage {
                return Err(VfsError::Invalid);
            }
            // A stage is entered only at its own root.
            if pair[0].stage == 0 && pair[1].stage != 0 && pair[1].stage != pair[1].object {
                return Err(VfsError::Invalid);
            }
        }
        Ok(Self {
            uuid,
            scope,
            generation,
            blocked,
            frames,
        })
    }
    fn encode(&self) -> Vec<u8> {
        let mut out = b"RRC1".to_vec();
        out.extend(self.uuid);
        for value in [
            self.scope,
            self.generation,
            self.blocked,
            self.frames.len() as u64,
        ] {
            out.extend(value.to_le_bytes());
        }
        for f in &self.frames {
            for value in [f.parent, f.object, f.ordinal, f.stage, f.name.len() as u64] {
                out.extend(value.to_le_bytes());
            }
            out.extend(f.name.as_bytes());
        }
        out.extend(afsplus_format::crc32c::crc32c(&out).to_le_bytes());
        out
    }
    fn popped(&mut self, removed: bool) {
        self.frames.pop();
        if removed {
            if let Some(parent) = self.frames.last_mut() {
                parent.ordinal = parent.ordinal.saturating_sub(1);
            }
        }
    }
    fn block(&mut self, object: u64, report: &mut RestoreRecoveryReport) {
        self.blocked = self.blocked.saturating_add(1);
        report.blocked_object = object;
        let stage = self.frames.last().map_or(0, |f| f.stage);
        if stage != 0 {
            while self.frames.last().is_some_and(|f| f.stage == stage) {
                self.frames.pop();
            }
        }
    }
}

impl<D: BlockDevice> Vfs<D> {
    // None is ordinary/inactive metadata; malformed values never authorize removal.
    fn recovery_marker(&mut self, object: u64) -> Result<Option<u64>, VfsError> {
        let Some(value) = self
            .volume
            .attribute(object, snapshot_restore::STAGE_ATTRIBUTE)?
        else {
            return Ok(None);
        };
        let raw: [u8; 8] = value.try_into().map_err(|_| VfsError::Invalid)?;
        let root = u64::from_le_bytes(raw);
        if !(afsplus_format::OBJECT_FIRST_DYNAMIC..self.volume.checkpoint().next_object_id)
            .contains(&root)
        {
            return Err(VfsError::Invalid);
        }
        match self
            .volume
            .attribute(root, snapshot_restore::STAGE_ATTRIBUTE)
        {
            Err(CoreError::NotFound) => Ok(None),
            Err(e) => Err(e.into()),
            Ok(None) => Ok(None),
            Ok(Some(v)) if v == root.to_le_bytes() => Ok(Some(root)),
            Ok(Some(_)) => Err(VfsError::Invalid),
        }
    }

    /// One explicit maintenance step. Keep the cursor outside this volume.
    /// Only exact active markers authorize reclaim; names bind entries, never
    /// identify candidates. Ordinary namespace changes require a safe restart.
    pub fn restore_reclaim_step(
        &mut self,
        scope: ObjectId,
        input: &[u8],
        limits: RestoreRecoveryLimits,
        now: Timespec,
    ) -> Result<RestoreRecoveryReport, VfsError> {
        if limits.scan_entries == 0
            || limits.scan_entries > 64
            || limits.work_steps == 0
            || limits.work_steps > 8
            || limits.max_depth == 0
            || limits.max_depth > MAX_DEPTH
        {
            return Err(VfsError::Invalid);
        }
        if self.mount_mode() != MountMode::ReadWrite {
            return Err(VfsError::ReadOnly);
        }
        if self.volume.ident().features.incompat
            & afsplus_format::ident::INCOMPAT_PERSISTENT_SNAPSHOTS
            == 0
        {
            return Err(VfsError::NotSupported);
        }
        self.ordinary_access(scope)?;
        // Do not run unrelated idle reclaim or orphan maintenance here: it
        // would spend an unreported budget and invalidate a quiescent cursor.
        if self.volume.window_open() {
            return Err(VfsError::Busy);
        }
        if self
            .volume
            .stat(scope)?
            .ok_or(VfsError::NotFound)?
            .object_type
            != ObjectType::Directory
        {
            return Err(VfsError::NotDirectory);
        }
        let mut cursor = if input.is_empty() {
            Cursor {
                uuid: self.volume.ident().uuid,
                scope,
                generation: self.volume.checkpoint().generation,
                blocked: 0,
                frames: vec![Frame {
                    parent: 0,
                    object: scope,
                    ordinal: 0,
                    stage: 0,
                    name: String::new(),
                }],
            }
        } else {
            Cursor::decode(input)?
        };
        if cursor.uuid != self.volume.ident().uuid
            || cursor.scope != scope
            || cursor.generation != self.volume.checkpoint().generation
        {
            return Err(VfsError::Stale);
        }
        if cursor.frames.len() > limits.max_depth + 1 {
            return Err(VfsError::Invalid);
        }
        for frame in cursor.frames.iter().skip(1) {
            if self.volume.lookup_in_directory(frame.parent, &frame.name)? != Some(frame.object) {
                return Err(VfsError::Stale);
            }
            if frame.stage != 0 && self.recovery_marker(frame.object)? != Some(frame.stage) {
                return Err(VfsError::Stale);
            }
        }
        let mut report = RestoreRecoveryReport::default();
        let mut work = 0;
        while let Some(frame) = cursor.frames.last().cloned() {
            let stat = self.volume.stat(frame.object)?.ok_or(VfsError::Stale)?;
            if stat.object_type == ObjectType::File {
                if work >= limits.work_steps {
                    break;
                }
                if frame.stage == 0 {
                    return Err(VfsError::Invalid);
                }
                if stat.link_count != 1 {
                    cursor.block(frame.object, &mut report);
                    continue;
                }
                if handle_holds(&self.handles, frame.object) {
                    cursor.block(frame.object, &mut report);
                    continue;
                }
                let step = self.volume.discard_restore_file_step(
                    frame.parent,
                    &frame.name,
                    frame.object,
                    frame.stage,
                    1,
                    now,
                )?;
                work += 1;
                if step.object_removed {
                    report.removed += 1;
                    cursor.popped(true);
                } else {
                    report.advanced += 1;
                }
                continue;
            }
            if stat.object_type != ObjectType::Directory {
                return Err(VfsError::Invalid);
            }
            if report.scanned as usize >= limits.scan_entries {
                break;
            }
            let page = self.volume.read_directory_page(
                frame.object,
                Some(DirectoryCursor {
                    generation: self.volume.checkpoint().generation,
                    ordinal: frame.ordinal,
                }),
                1,
            )?;
            if let Some(entry) = page.entries.into_iter().next() {
                report.scanned += 1;
                cursor.frames.last_mut().unwrap().ordinal =
                    frame.ordinal.checked_add(1).ok_or(VfsError::Invalid)?;
                let marker = match self.recovery_marker(entry.child_id) {
                    Ok(m) => m,
                    Err(VfsError::Invalid) => {
                        cursor.block(entry.child_id, &mut report);
                        continue;
                    }
                    Err(e) => return Err(e),
                };
                let stage = if frame.stage != 0 {
                    if marker != Some(frame.stage) {
                        cursor.block(entry.child_id, &mut report);
                        continue;
                    }
                    frame.stage
                } else {
                    match marker {
                        Some(root) if root == entry.child_id => root,
                        Some(_) => {
                            cursor.block(entry.child_id, &mut report);
                            continue;
                        }
                        None => 0,
                    }
                };
                let child = self.volume.stat(entry.child_id)?.ok_or(VfsError::Stale)?;
                if child.object_type == ObjectType::Directory
                    || (stage != 0 && child.object_type == ObjectType::File)
                {
                    // A directory below the depth bound is reported, not
                    // skipped: its id is the deeper scope to select next.
                    if cursor.frames.len() > limits.max_depth
                        || cursor.frames.iter().any(|f| f.object == entry.child_id)
                    {
                        cursor.block(entry.child_id, &mut report);
                        continue;
                    }
                    let Ok(name) = String::from_utf8(entry.name) else {
                        cursor.block(entry.child_id, &mut report);
                        continue;
                    };
                    cursor.frames.push(Frame {
                        parent: frame.object,
                        object: entry.child_id,
                        ordinal: 0,
                        stage,
                        name,
                    });
                } else if stage != 0 {
                    cursor.block(entry.child_id, &mut report);
                }
            } else if frame.stage != 0 {
                if work >= limits.work_steps {
                    break;
                }
                if self.recovery_marker(frame.object)? != Some(frame.stage)
                    || self.volume.lookup_in_directory(frame.parent, &frame.name)?
                        != Some(frame.object)
                {
                    return Err(VfsError::Stale);
                }
                self.volume
                    .remove_directory(frame.parent, &frame.name, now)?;
                work += 1;
                report.removed += 1;
                cursor.popped(true);
            } else {
                cursor.popped(false);
            }
        }
        report.done = cursor.frames.is_empty();
        report.blocked = cursor.blocked;
        cursor.generation = self.volume.checkpoint().generation;
        if !report.done {
            report.cursor = cursor.encode();
        }
        Ok(report)
    }
}
