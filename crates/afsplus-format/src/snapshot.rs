//! Fixed-size leaf codecs for the ADR-072 snapshot experiment.
//! Enclosing AFST headers supply CRC, owner, generation and ordering checks.
//! Tree/bitmap ownership and feature negotiation are caller obligations.
use crate::{le, FormatError};

pub const VALUE_SIZE: usize = 32;

fn fields(value: &[u8], used: usize) -> Result<(), FormatError> {
    if value.len() != VALUE_SIZE {
        return Err(FormatError::Invalid("snapshot value length"));
    }
    if value[used..].iter().any(|&byte| byte != 0) {
        return Err(FormatError::Invalid("snapshot reserved bytes"));
    }
    Ok(())
}

pub fn decode_key(key: &[u8]) -> Result<u64, FormatError> {
    let bytes: [u8; 8] = key
        .try_into()
        .map_err(|_| FormatError::Invalid("snapshot key length"))?;
    Ok(u64::from_be_bytes(bytes))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegistryState {
    pub next_id: u64,
}

impl RegistryState {
    pub fn encode(self) -> Result<[u8; VALUE_SIZE], FormatError> {
        if self.next_id == 0 {
            return Err(FormatError::Invalid("snapshot next ID is zero"));
        }
        let mut value = [0; VALUE_SIZE];
        le::put_u64(&mut value[..8], self.next_id);
        Ok(value)
    }

    pub fn decode(value: &[u8]) -> Result<Self, FormatError> {
        fields(value, 8)?;
        let state = Self {
            next_id: le::get_u64(&value[..8]),
        };
        state.encode()?;
        Ok(state)
    }

    /// Returns the ID and updated control state; exhaustion never wraps.
    pub fn allocate_id(self) -> Result<(u64, Self), FormatError> {
        self.encode()?;
        let next_id = self
            .next_id
            .checked_add(1)
            .ok_or(FormatError::Overflow("snapshot ID exhausted"))?;
        Ok((self.next_id, Self { next_id }))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotRecord {
    pub generation: u64,
    pub committed_tx_id: u64,
    pub object_map_root: u64,
}

impl SnapshotRecord {
    pub fn validate(self, max_generation: u64, total_blocks: u64) -> Result<(), FormatError> {
        if self.generation == 0 || self.generation > max_generation {
            return Err(FormatError::Invalid("snapshot captured generation"));
        }
        if self.committed_tx_id == 0 || self.committed_tx_id > self.generation {
            return Err(FormatError::Invalid("snapshot committed transaction ID"));
        }
        if self.object_map_root == 0 || self.object_map_root >= total_blocks {
            return Err(FormatError::Invalid("snapshot object-map root"));
        }
        Ok(())
    }

    pub fn encode(
        self,
        max_generation: u64,
        total_blocks: u64,
    ) -> Result<[u8; VALUE_SIZE], FormatError> {
        self.validate(max_generation, total_blocks)?;
        let mut value = [0; VALUE_SIZE];
        le::put_u64(&mut value[..8], self.generation);
        le::put_u64(&mut value[8..16], self.committed_tx_id);
        le::put_u64(&mut value[16..24], self.object_map_root);
        Ok(value)
    }

    pub fn decode(
        value: &[u8],
        max_generation: u64,
        total_blocks: u64,
    ) -> Result<Self, FormatError> {
        fields(value, 24)?;
        let record = Self {
            generation: le::get_u64(&value[..8]),
            committed_tx_id: le::get_u64(&value[8..16]),
            object_map_root: le::get_u64(&value[16..24]),
        };
        record.validate(max_generation, total_blocks)?;
        Ok(record)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LifetimeRecord {
    pub blocks: u64,
    pub birth: u64,
    /// Zero while live; otherwise the first generation without a live owner.
    pub retirement: u64,
}

impl LifetimeRecord {
    pub fn validate(
        self,
        start: u64,
        max_generation: u64,
        total_blocks: u64,
    ) -> Result<(), FormatError> {
        if start == 0
            || self.blocks == 0
            || start
                .checked_add(self.blocks)
                .is_none_or(|end| end > total_blocks)
        {
            return Err(FormatError::Invalid("snapshot lifetime physical range"));
        }
        if self.birth == 0 || self.birth > max_generation {
            return Err(FormatError::Invalid("snapshot lifetime birth"));
        }
        if self.retirement != 0
            && (self.retirement <= self.birth || self.retirement > max_generation)
        {
            return Err(FormatError::Invalid("snapshot lifetime retirement"));
        }
        Ok(())
    }

    pub fn encode(
        self,
        start: u64,
        max_generation: u64,
        total_blocks: u64,
    ) -> Result<[u8; VALUE_SIZE], FormatError> {
        self.validate(start, max_generation, total_blocks)?;
        let mut value = [0; VALUE_SIZE];
        le::put_u64(&mut value[..8], self.blocks);
        le::put_u64(&mut value[8..16], self.birth);
        le::put_u64(&mut value[16..24], self.retirement);
        Ok(value)
    }

    pub fn decode(
        value: &[u8],
        start: u64,
        max_generation: u64,
        total_blocks: u64,
    ) -> Result<Self, FormatError> {
        fields(value, 24)?;
        let record = Self {
            blocks: le::get_u64(&value[..8]),
            birth: le::get_u64(&value[8..16]),
            retirement: le::get_u64(&value[16..24]),
        };
        record.validate(start, max_generation, total_blocks)?;
        Ok(record)
    }

    /// Use only after contextual validation; live lifetimes have no upper bound.
    pub fn contains(self, generation: u64) -> bool {
        self.birth <= generation && (self.retirement == 0 || generation < self.retirement)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LedgerState {
    pub scan_position: u64,
    pub retained_blocks: u64,
}

impl LedgerState {
    pub fn encode(self, total_blocks: u64) -> Result<[u8; VALUE_SIZE], FormatError> {
        if total_blocks == 0
            || self.scan_position >= total_blocks
            || self.retained_blocks > total_blocks
        {
            return Err(FormatError::Invalid("snapshot ledger control range"));
        }
        let mut value = [0; VALUE_SIZE];
        le::put_u64(&mut value[..8], self.scan_position);
        le::put_u64(&mut value[8..16], self.retained_blocks);
        Ok(value)
    }

    pub fn decode(value: &[u8], total_blocks: u64) -> Result<Self, FormatError> {
        fields(value, 16)?;
        let state = Self {
            scan_position: le::get_u64(&value[..8]),
            retained_blocks: le::get_u64(&value[8..16]),
        };
        state.encode(total_blocks)?;
        Ok(state)
    }
}
