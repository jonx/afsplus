//! Deterministic classic-format reservation images for independent C reads.
use afsplus_block::{BlockDevice, MemoryBackend};
use afsplus_core::{mkfs, mount, MkfsParams, NamePolicy};
use afsplus_format::Timespec;
use std::{error::Error, fs::File, io::Write, path::Path};
fn time(n: i64) -> Timespec {
    Timespec {
        seconds: n,
        nanoseconds: 0,
    }
}
fn save(dir: &Path, name: &str, dev: &MemoryBackend) -> Result<(), Box<dyn Error>> {
    let mut file = File::create_new(dir.join(name))?;
    for block in 0..dev.total_blocks() {
        file.write_all(&dev.peek(block))?;
    }
    Ok(())
}
fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args_os().skip(1);
    let output = args
        .next()
        .ok_or("usage: afsplus-reservation-fixture NEW_DIRECTORY")?;
    if args.next().is_some() {
        return Err("expected one new output directory".into());
    }
    let dir = Path::new(&output);
    std::fs::create_dir(dir)?;
    let mut dev = MemoryBackend::new(4096, 256);
    mkfs(
        &mut dev,
        &MkfsParams {
            uuid: [91; 16],
            label: "ReservationC".into(),
            region_size: 256,
            reclaim_caps: Default::default(),
            log_slots: 0,
            shared_extents: false,
            data_policy: false,
            name_policy: NamePolicy::Sensitive,
            timestamp: time(1),
        },
    )?;
    let mut volume = mount(dev)?;
    let file = volume.create_file_in_root("reserved", &[], time(2))?;
    volume.preallocate_file(file, 0, 16384, time(3))?;
    volume.truncate_file(file, 16384, time(4))?;
    let old_generation = volume.generation();
    let before = volume.into_device();
    save(dir, "before.afsp", &before)?;
    let mut volume = mount(before)?;
    volume.write_file_at(file, 7, &vec![0x5a; 5000], time(5))?;
    assert_eq!(
        volume
            .last_commit_stats()
            .unwrap()
            .data_blocks_initialized_from_reservation,
        2
    );
    let generation = volume.generation();
    let slots = volume.ident().checkpoint_slots;
    let after = volume.into_device();
    save(dir, "after.afsp", &after)?;
    let mut fallback = after;
    let mut invalidated = 0;
    for slot in slots {
        let mut bytes = fallback.peek(slot);
        if afsplus_format::checkpoint::Checkpoint::decode(&bytes, &[91; 16])?.generation
            == generation
        {
            bytes[28] ^= 1;
            fallback.write_block(slot, &bytes)?;
            invalidated += 1;
        }
    }
    assert_eq!(invalidated, 1);
    let mut reopened = mount(fallback.clone())?;
    assert_eq!(reopened.generation(), old_generation);
    assert_eq!(reopened.read_file(file)?, vec![0; 16384]);
    save(dir, "fallback.afsp", &fallback)?;
    for name in ["before.afsp", "after.afsp", "fallback.afsp"] {
        println!("fixture={name} logical_size=16384 initialized_range=7..5007");
    }
    Ok(())
}
