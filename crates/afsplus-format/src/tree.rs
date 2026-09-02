//! Generic checksummed node format for the bounded COW B+ tree prototype.
//!
//! Typed object-map, directory, extent, and allocation-root adapters own their
//! key/value encodings. This module owns only structural tree invariants.

use alloc::vec;
use alloc::vec::Vec;

use crate::header::{block_type, BlockHeader, HEADER_SIZE};
use crate::{le, FormatError};

const FIXED_PAYLOAD: usize = 32;
const ITEM_FIXED: usize = 8;

pub const MAX_TREE_LEVEL: u8 = 15;
pub const MAX_TREE_KEY_BYTES: usize = 1020;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeKind {
    ObjectMap,
    Directory,
    ExtentMap,
    AllocationRoot,
    /// Volume-wide shared-extent reference tree (ADR-061); owner 0.
    SharedExtents,
}

impl TreeKind {
    fn to_wire(self) -> u8 {
        match self {
            TreeKind::ObjectMap => 1,
            TreeKind::Directory => 2,
            TreeKind::ExtentMap => 3,
            TreeKind::AllocationRoot => 4,
            TreeKind::SharedExtents => 5,
        }
    }

    fn from_wire(value: u8) -> Result<Self, FormatError> {
        match value {
            1 => Ok(TreeKind::ObjectMap),
            2 => Ok(TreeKind::Directory),
            3 => Ok(TreeKind::ExtentMap),
            4 => Ok(TreeKind::AllocationRoot),
            5 => Ok(TreeKind::SharedExtents),
            _ => Err(FormatError::Invalid("unknown tree kind")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeItem {
    pub key: Vec<u8>,
    /// Leaf value, or a 16-byte child reference in an internal node.
    pub value: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChildRef {
    pub lba: u64,
    pub subtree_items: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeNode {
    pub kind: TreeKind,
    pub owner: u64,
    pub level: u8,
    /// Exact number of leaf items reachable from this node.
    pub subtree_items: u64,
    /// Internal-node child for keys below the first separator; zero in leaves.
    pub leftmost_child: u64,
    /// Items below `leftmost_child`; zero in leaves.
    pub leftmost_items: u64,
    pub items: Vec<TreeItem>,
}

impl TreeNode {
    pub fn leaf(kind: TreeKind, owner: u64) -> Self {
        TreeNode {
            kind,
            owner,
            level: 0,
            subtree_items: 0,
            leftmost_child: 0,
            leftmost_items: 0,
            items: Vec::new(),
        }
    }

    pub fn is_leaf(&self) -> bool {
        self.level == 0
    }

    /// Number of fixed-size items that fit in one encoded node. Typed tree
    /// adapters use this instead of constructing and repeatedly growing a
    /// temporary node merely to discover its capacity.
    pub fn fixed_item_capacity(
        block_size: usize,
        key_len: usize,
        value_len: usize,
    ) -> Result<usize, FormatError> {
        if key_len == 0 || key_len > MAX_TREE_KEY_BYTES || value_len == 0 {
            return Err(FormatError::Invalid("tree item length out of range"));
        }
        let item_len = ITEM_FIXED
            .checked_add(key_len)
            .and_then(|length| length.checked_add(value_len))
            .ok_or(FormatError::Overflow("tree item length"))?;
        let available = block_size
            .checked_sub(HEADER_SIZE + FIXED_PAYLOAD)
            .ok_or(FormatError::Overflow("tree node fixed payload"))?;
        Ok(available / item_len)
    }

    pub fn encoded_len(&self) -> Result<usize, FormatError> {
        let mut length = FIXED_PAYLOAD;
        for item in &self.items {
            length = length
                .checked_add(ITEM_FIXED)
                .and_then(|value| value.checked_add(item.key.len()))
                .and_then(|value| value.checked_add(item.value.len()))
                .ok_or(FormatError::Overflow("tree node payload length"))?;
        }
        Ok(length)
    }

    pub fn fits(&self, block_size: usize) -> bool {
        self.encoded_len()
            .is_ok_and(|length| length <= block_size.saturating_sub(HEADER_SIZE))
    }

    pub fn child_ref(item: &TreeItem) -> Result<ChildRef, FormatError> {
        if item.value.len() != 16 {
            return Err(FormatError::Invalid("internal tree child has wrong size"));
        }
        let child = ChildRef {
            lba: le::get_u64(&item.value[0..8]),
            subtree_items: le::get_u64(&item.value[8..16]),
        };
        if child.lba == 0 || child.subtree_items == 0 {
            return Err(FormatError::Invalid("internal tree child is zero"));
        }
        Ok(child)
    }

    pub fn encode(
        &self,
        block_size: usize,
        transaction_generation: u64,
    ) -> Result<Vec<u8>, FormatError> {
        if transaction_generation == 0 {
            return Err(FormatError::Invalid("tree generation must be nonzero"));
        }
        self.validate()?;
        let payload_len = self.encoded_len()?;
        if payload_len > block_size.saturating_sub(HEADER_SIZE) {
            return Err(FormatError::Overflow("tree node exceeds one block"));
        }
        if self.items.len() > u32::MAX as usize {
            return Err(FormatError::Overflow("tree node item count"));
        }

        let mut block = vec![0u8; block_size];
        let p = &mut block[HEADER_SIZE..];
        p[0] = self.kind.to_wire();
        p[1] = self.level;
        le::put_u32(&mut p[4..8], self.items.len() as u32);
        le::put_u64(&mut p[8..16], self.subtree_items);
        le::put_u64(&mut p[16..24], self.leftmost_child);
        le::put_u64(&mut p[24..32], self.leftmost_items);
        let mut offset = FIXED_PAYLOAD;
        for item in &self.items {
            if item.key.len() > u16::MAX as usize || item.value.len() > u16::MAX as usize {
                return Err(FormatError::Overflow("tree item length"));
            }
            le::put_u16(&mut p[offset..offset + 2], item.key.len() as u16);
            le::put_u16(&mut p[offset + 2..offset + 4], item.value.len() as u16);
            offset += ITEM_FIXED;
            p[offset..offset + item.key.len()].copy_from_slice(&item.key);
            offset += item.key.len();
            p[offset..offset + item.value.len()].copy_from_slice(&item.value);
            offset += item.value.len();
        }

        BlockHeader {
            block_type: block_type::TREE_NODE,
            flags: 0,
            owner: self.owner,
            generation: transaction_generation,
            payload_len: payload_len as u32,
        }
        .seal(&mut block);
        Ok(block)
    }

    pub fn decode(block: &[u8]) -> Result<(TreeNode, u64), FormatError> {
        let header = BlockHeader::verify(block, block_type::TREE_NODE)?;
        let p = header.payload(block);
        if p.len() < FIXED_PAYLOAD {
            return Err(FormatError::Invalid("tree node payload too short"));
        }
        if header.flags != 0 || header.generation == 0 || p[2..4] != [0, 0] {
            return Err(FormatError::Invalid("tree node header fields are invalid"));
        }
        let count = le::get_u32(&p[4..8]) as usize;
        if count > (p.len() - FIXED_PAYLOAD) / ITEM_FIXED {
            return Err(FormatError::Invalid("tree item count exceeds payload"));
        }
        let mut items = Vec::with_capacity(count);
        let mut offset = FIXED_PAYLOAD;
        for _ in 0..count {
            if p.len() - offset < ITEM_FIXED {
                return Err(FormatError::Invalid("truncated tree item"));
            }
            let key_len = le::get_u16(&p[offset..offset + 2]) as usize;
            let value_len = le::get_u16(&p[offset + 2..offset + 4]) as usize;
            if p[offset + 4..offset + 8] != [0, 0, 0, 0] {
                return Err(FormatError::Invalid("tree item reserved field is nonzero"));
            }
            offset += ITEM_FIXED;
            if key_len > p.len() - offset || value_len > p.len() - offset - key_len {
                return Err(FormatError::Invalid("tree item lengths exceed payload"));
            }
            let key = p[offset..offset + key_len].to_vec();
            offset += key_len;
            let value = p[offset..offset + value_len].to_vec();
            offset += value_len;
            items.push(TreeItem { key, value });
        }
        if offset != p.len() {
            return Err(FormatError::Invalid("tree node payload length mismatch"));
        }
        let node = TreeNode {
            kind: TreeKind::from_wire(p[0])?,
            owner: header.owner,
            level: p[1],
            subtree_items: le::get_u64(&p[8..16]),
            leftmost_child: le::get_u64(&p[16..24]),
            leftmost_items: le::get_u64(&p[24..32]),
            items,
        };
        node.validate()?;
        Ok((node, header.generation))
    }

    fn validate(&self) -> Result<(), FormatError> {
        if self.level > MAX_TREE_LEVEL {
            return Err(FormatError::Invalid(
                "tree level exceeds implementation cap",
            ));
        }
        for item in &self.items {
            if item.key.is_empty() || item.key.len() > MAX_TREE_KEY_BYTES {
                return Err(FormatError::Invalid("tree key length out of range"));
            }
            if item.value.is_empty() {
                return Err(FormatError::Invalid("tree value is empty"));
            }
        }
        for pair in self.items.windows(2) {
            if pair[0].key >= pair[1].key {
                return Err(FormatError::Invalid("tree keys not strictly ordered"));
            }
        }
        if self.is_leaf() {
            if self.leftmost_child != 0
                || self.leftmost_items != 0
                || self.subtree_items != self.items.len() as u64
            {
                return Err(FormatError::Invalid("leaf tree accounting is inconsistent"));
            }
        } else {
            if self.leftmost_child == 0 || self.leftmost_items == 0 || self.items.is_empty() {
                return Err(FormatError::Invalid("internal tree node has no children"));
            }
            let mut counted = self.leftmost_items;
            for item in &self.items {
                counted = counted
                    .checked_add(Self::child_ref(item)?.subtree_items)
                    .ok_or(FormatError::Overflow("tree subtree item count"))?;
            }
            if self.subtree_items != counted {
                return Err(FormatError::Invalid(
                    "internal tree item count is inconsistent",
                ));
            }
        }
        Ok(())
    }
}

/// Fixed-width unsigned integer key whose byte ordering matches numeric order.
pub fn key_u64(value: u64) -> [u8; 8] {
    value.to_be_bytes()
}

/// Fixed-width unsigned integer key whose byte ordering matches numeric order.
pub fn key_u32(value: u32) -> [u8; 4] {
    value.to_be_bytes()
}

pub fn child_value(child: ChildRef) -> Result<Vec<u8>, FormatError> {
    if child.lba == 0 || child.subtree_items == 0 {
        return Err(FormatError::Invalid("internal tree child is zero"));
    }
    let mut value = vec![0u8; 16];
    le::put_u64(&mut value[0..8], child.lba);
    le::put_u64(&mut value[8..16], child.subtree_items);
    Ok(value)
}
