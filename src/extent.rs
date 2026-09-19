//! Extent descriptors and mapping for high-throughput contiguous I/O.

use crate::types::BlockId;

/// Size of an on-disk extent descriptor in bytes:
/// logical_block (4) + physical_block (8) + length (4) + flags (4) = 20 bytes.
pub const EXTENT_DESCRIPTOR_SIZE: usize = 20;

/// Number of direct extents embedded in each inode record.
pub const DIRECT_EXTENTS_COUNT: usize = 5;

/// Flags for extents.
pub const EXTENT_FLAG_COW: u32 = 0x0001;     // Copy-on-Write shared extent
pub const EXTENT_FLAG_UNWRITTEN: u32 = 0x0002; // Pre-allocated but not written

/// An Extent represents a contiguous run of physical blocks on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Extent {
    pub logical_block: u32,   // Logical block offset within file
    pub physical_block: BlockId, // Starting physical block on device
    pub length: u32,          // Number of contiguous blocks in this extent
    pub flags: u32,           // Extent flags (e.g. CoW, Unwritten)
}

impl Extent {
    pub const fn empty() -> Self {
        Self {
            logical_block: 0,
            physical_block: 0,
            length: 0,
            flags: 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.length == 0
    }

    pub fn contains_logical(&self, logical: u32) -> bool {
        !self.is_empty()
            && logical >= self.logical_block
            && logical < self.logical_block + self.length
    }

    /// Serialize into a 20-byte slice.
    pub fn serialize(&self, buf: &mut [u8]) {
        buf[0..4].copy_from_slice(&self.logical_block.to_le_bytes());
        buf[4..12].copy_from_slice(&self.physical_block.to_le_bytes());
        buf[12..16].copy_from_slice(&self.length.to_le_bytes());
        buf[16..20].copy_from_slice(&self.flags.to_le_bytes());
    }

    /// Deserialize from a 20-byte slice.
    pub fn deserialize(buf: &[u8]) -> Self {
        let logical_block = u32::from_le_bytes(buf[0..4].try_into().unwrap());
        let physical_block = u64::from_le_bytes(buf[4..12].try_into().unwrap());
        let length = u32::from_le_bytes(buf[12..16].try_into().unwrap());
        let flags = u32::from_le_bytes(buf[16..20].try_into().unwrap());
        Self {
            logical_block,
            physical_block,
            length,
            flags,
        }
    }

    /// Translate a logical block index to its physical block ID.
    pub fn map_block(&self, logical: u32) -> Option<BlockId> {
        if self.contains_logical(logical) {
            let offset = (logical - self.logical_block) as u64;
            Some(self.physical_block + offset)
        } else {
            None
        }
    }
}
