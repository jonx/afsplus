//! Direct selection in the existing bounded full-subset/representative-tear model.
use afsplus_block::{BlockDevice, MemoryBackend, RecordedOp};

/// Variant order matches afsplus-block's streaming crash enumerator.
/// Refuse an inadmissible cut or variant without modifying the supplied base.
pub fn select(
    base: &MemoryBackend,
    log: &[RecordedOp],
    cut: usize,
    variant: usize,
) -> Result<MemoryBackend, String> {
    let prefix = log.get(..cut).ok_or("crash cut outside trace")?;
    let durable_end = prefix
        .iter()
        .rposition(|op| matches!(op, RecordedOp::Flush))
        .map_or(0, |i| i + 1);
    let tail: Vec<_> = prefix[durable_end..]
        .iter()
        .filter_map(|op| match op {
            RecordedOp::Write { lba, data } => Some((*lba, data)),
            RecordedOp::Flush => None,
        })
        .take(13)
        .collect();
    if tail.len() > 12 {
        return Err("crash tail exceeds exhaustive model".into());
    }
    let tears: Vec<_> = [64, 2048, 4064]
        .into_iter()
        .filter(|&n| n < base.block_size())
        .collect();
    let subsets = 1usize << tail.len();
    if variant >= subsets + tail.len() * tears.len() {
        return Err("crash variant outside model".into());
    }
    for op in prefix {
        if let RecordedOp::Write { lba, data } = op {
            if *lba >= base.total_blocks() || data.len() != base.block_size() {
                return Err("crash trace geometry".into());
            }
        }
    }
    let mut image = base.clone();
    for op in &prefix[..durable_end] {
        if let RecordedOp::Write { lba, data } = op {
            image.write_block(*lba, data).map_err(|e| e.to_string())?;
        }
    }
    if variant < subsets {
        for (index, (lba, data)) in tail.iter().enumerate() {
            if variant & (1 << index) != 0 {
                image.write_block(*lba, data).map_err(|e| e.to_string())?;
            }
        }
    } else {
        let choice = variant - subsets;
        let index = choice / tears.len();
        for (lba, data) in &tail[..index] {
            image.write_block(*lba, data).map_err(|e| e.to_string())?;
        }
        let (lba, data) = tail[index];
        let tear = tears[choice % tears.len()];
        let mut block = image.peek(lba);
        block[..tear].copy_from_slice(&data[..tear]);
        image.write_block(lba, &block).map_err(|e| e.to_string())?;
    }
    Ok(image)
}
