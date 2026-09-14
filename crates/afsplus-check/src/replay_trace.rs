//! ADR-098 host replay artifacts. Decoding and identity checks never write a device.
use afsplus_block::{BlockDevice, RecordedOp};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy)]
pub struct Limits {
    pub wire_bytes: usize,
    pub operations: usize,
    pub block_size: usize,
    pub base_blocks: u64,
}
pub struct Trace {
    pub block_size: usize,
    pub blocks: u64,
    pub base_digest: [u8; 32],
    pub operations: Vec<RecordedOp>,
}
fn geometry(bs: usize, blocks: u64, limits: Limits) -> Result<(), String> {
    if bs < 512
        || !bs.is_power_of_two()
        || bs > limits.block_size
        || bs > u32::MAX as usize
        || blocks == 0
        || blocks > limits.base_blocks
    {
        return Err("trace geometry exceeds admission".into());
    }
    Ok(())
}
pub fn base_digest<D: BlockDevice>(device: &mut D, limits: Limits) -> Result<[u8; 32], String> {
    let bs = device.block_size();
    let blocks = device.total_blocks();
    geometry(bs, blocks, limits)?;
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(bs)
        .map_err(|_| "base buffer allocation")?;
    buffer.resize(bs, 0);
    let mut digest = Sha256::new();
    digest.update(b"AFS+ replay base v1\0");
    digest.update((bs as u32).to_le_bytes());
    digest.update(blocks.to_le_bytes());
    for lba in 0..blocks {
        device
            .read_block(lba, &mut buffer)
            .map_err(|e| e.to_string())?;
        digest.update(&buffer);
    }
    Ok(digest.finalize().into())
}
impl Trace {
    pub fn verify_base<D: BlockDevice>(
        &self,
        device: &mut D,
        limits: Limits,
    ) -> Result<(), String> {
        if device.block_size() != self.block_size || device.total_blocks() != self.blocks {
            return Err("trace base geometry mismatch".into());
        }
        if base_digest(device, limits)? != self.base_digest {
            return Err("trace base digest mismatch".into());
        }
        Ok(())
    }
    pub fn encode(&self, limits: Limits) -> Result<Vec<u8>, String> {
        geometry(self.block_size, self.blocks, limits)?;
        if self.operations.len() > limits.operations {
            return Err("trace operation limit".into());
        }
        let mut length = 96usize;
        for op in &self.operations {
            let size = match op {
                RecordedOp::Flush => 1,
                RecordedOp::Write { lba, data } => {
                    if *lba >= self.blocks || data.len() != self.block_size {
                        return Err("invalid trace write".into());
                    }
                    9usize
                        .checked_add(data.len())
                        .ok_or("trace size overflow")?
                }
            };
            length = length.checked_add(size).ok_or("trace size overflow")?;
        }
        if length > limits.wire_bytes {
            return Err("trace wire limit".into());
        }
        let mut wire = Vec::new();
        wire.try_reserve_exact(length)
            .map_err(|_| "trace wire allocation")?;
        wire.extend_from_slice(b"AFSTRC00");
        wire.extend_from_slice(&1u32.to_le_bytes());
        wire.extend_from_slice(&(self.block_size as u32).to_le_bytes());
        wire.extend_from_slice(&self.blocks.to_le_bytes());
        wire.extend_from_slice(&(self.operations.len() as u64).to_le_bytes());
        wire.extend_from_slice(&self.base_digest);
        for op in &self.operations {
            match op {
                RecordedOp::Flush => wire.push(2),
                RecordedOp::Write { lba, data } => {
                    wire.push(1);
                    wire.extend_from_slice(&lba.to_le_bytes());
                    wire.extend_from_slice(data);
                }
            }
        }
        let digest = Sha256::digest(&wire);
        wire.extend_from_slice(&digest);
        Ok(wire)
    }
    pub fn decode(wire: &[u8], limits: Limits) -> Result<Self, String> {
        if wire.len() < 96 || wire.len() > limits.wire_bytes {
            return Err("trace wire limit".into());
        }
        let (body, checksum) = wire.split_at(wire.len() - 32);
        if Sha256::digest(body).as_slice() != checksum {
            return Err("trace checksum mismatch".into());
        }
        let mut cursor = body;
        if take(&mut cursor, 8)? != b"AFSTRC00" || number::<4>(&mut cursor)? != 1 {
            return Err("unknown trace format".into());
        }
        let block_size =
            usize::try_from(number::<4>(&mut cursor)?).map_err(|_| "trace block size")?;
        let blocks = number::<8>(&mut cursor)?;
        geometry(block_size, blocks, limits)?;
        let count = usize::try_from(number::<8>(&mut cursor)?).map_err(|_| "trace count")?;
        let base_digest = take(&mut cursor, 32)?
            .try_into()
            .map_err(|_| "base digest length")?;
        if count > limits.operations || count > cursor.len() {
            return Err("trace operation limit".into());
        }
        let mut operations = Vec::new();
        operations
            .try_reserve_exact(count)
            .map_err(|_| "trace operation allocation")?;
        for _ in 0..count {
            match take(&mut cursor, 1)?[0] {
                2 => operations.push(RecordedOp::Flush),
                1 => {
                    let lba = number::<8>(&mut cursor)?;
                    if lba >= blocks {
                        return Err("trace write outside base".into());
                    }
                    let payload = take(&mut cursor, block_size)?;
                    let mut data = Vec::new();
                    data.try_reserve_exact(block_size)
                        .map_err(|_| "trace payload allocation")?;
                    data.extend_from_slice(payload);
                    operations.push(RecordedOp::Write { lba, data });
                }
                _ => return Err("unknown trace operation".into()),
            }
        }
        if !cursor.is_empty() {
            return Err("trailing trace bytes".into());
        }
        Ok(Self {
            block_size,
            blocks,
            base_digest,
            operations,
        })
    }
}
fn take<'a>(cursor: &mut &'a [u8], count: usize) -> Result<&'a [u8], String> {
    if cursor.len() < count {
        return Err("truncated trace".into());
    }
    let (part, rest) = cursor.split_at(count);
    *cursor = rest;
    Ok(part)
}
fn number<const N: usize>(cursor: &mut &[u8]) -> Result<u64, String> {
    let mut bytes = [0; 8];
    bytes[..N].copy_from_slice(take(cursor, N)?);
    Ok(u64::from_le_bytes(bytes))
}
