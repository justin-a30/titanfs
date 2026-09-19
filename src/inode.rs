//! On-disk Inode structure, inline data handling, and extent management.
//! Exactly 256 bytes per inode, fitting 16 inodes per 4096-byte block.

use std::time::{SystemTime, UNIX_EPOCH};
use crate::crc::crc32c;
use crate::extent::Extent;
use crate::types::{
    BlockId, FileType, FsError, InodeId, Result, FLAG_DIRECTORY, FLAG_INLINE_DATA,
    INODE_SIZE,
};

pub const INLINE_DATA_CAPACITY: usize = 112;
pub const DIRECT_EXTENTS: usize = 3;

/// In-memory and on-disk Inode representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inode {
    pub id: InodeId,
    pub file_type: FileType,
    pub mode: u16,
    pub uid: u32,
    pub gid: u32,
    pub flags: u32,
    pub size: u64,
    pub allocated_blocks: u64,
    pub atime_ns: u64,
    pub mtime_ns: u64,
    pub ctime_ns: u64,
    pub link_count: u32,
    pub inline_size: u16,
    pub indirect_extent_block: BlockId,
    pub inline_data: [u8; INLINE_DATA_CAPACITY],
    pub extents: [Extent; DIRECT_EXTENTS],
    pub crc: u32,
}

impl Inode {
    /// Create a new inode with current timestamp.
    pub fn new(id: InodeId, file_type: FileType, mode: u16) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;

        let flags = match file_type {
            FileType::Directory => FLAG_DIRECTORY,
            _ => FLAG_INLINE_DATA,
        };

        let mut inode = Self {
            id,
            file_type,
            mode,
            uid: 1000,
            gid: 1000,
            flags,
            size: 0,
            allocated_blocks: 0,
            atime_ns: now,
            mtime_ns: now,
            ctime_ns: now,
            link_count: 1,
            inline_size: 0,
            indirect_extent_block: 0,
            inline_data: [0u8; INLINE_DATA_CAPACITY],
            extents: [Extent::empty(); DIRECT_EXTENTS],
            crc: 0,
        };
        inode.crc = inode.compute_crc();
        inode
    }

    /// Check if inode is a directory.
    pub fn is_dir(&self) -> bool {
        self.file_type == FileType::Directory
    }

    /// Check if inode data is inlined.
    pub fn is_inline(&self) -> bool {
        (self.flags & FLAG_INLINE_DATA) != 0
    }

    /// Set or unset inline data flag.
    pub fn set_inline(&mut self, inline: bool) {
        if inline {
            self.flags |= FLAG_INLINE_DATA;
        } else {
            self.flags &= !FLAG_INLINE_DATA;
        }
    }

    /// Update modification and change timestamps to now.
    pub fn touch(&mut self) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        self.mtime_ns = now;
        self.ctime_ns = now;
    }

    /// Compute CRC32C over the first 252 bytes of the inode record.
    pub fn compute_crc(&self) -> u32 {
        let mut buf = [0u8; INODE_SIZE];
        self.serialize_internal(&mut buf);
        crc32c(&buf[..252])
    }

    fn serialize_internal(&self, buf: &mut [u8]) {
        buf.fill(0);
        buf[0..8].copy_from_slice(&self.id.to_le_bytes());
        buf[8] = self.file_type as u8;
        buf[9..11].copy_from_slice(&self.mode.to_le_bytes());
        buf[11] = 0; // reserved1
        buf[12..16].copy_from_slice(&self.uid.to_le_bytes());
        buf[16..20].copy_from_slice(&self.gid.to_le_bytes());
        buf[20..24].copy_from_slice(&self.flags.to_le_bytes());
        buf[24..32].copy_from_slice(&self.size.to_le_bytes());
        buf[32..40].copy_from_slice(&self.allocated_blocks.to_le_bytes());
        buf[40..48].copy_from_slice(&self.atime_ns.to_le_bytes());
        buf[48..56].copy_from_slice(&self.mtime_ns.to_le_bytes());
        buf[56..64].copy_from_slice(&self.ctime_ns.to_le_bytes());
        buf[64..68].copy_from_slice(&self.link_count.to_le_bytes());
        buf[68..70].copy_from_slice(&self.inline_size.to_le_bytes());
        buf[70..72].copy_from_slice(&0u16.to_le_bytes()); // reserved2
        buf[72..80].copy_from_slice(&self.indirect_extent_block.to_le_bytes());
        buf[80..192].copy_from_slice(&self.inline_data);

        for i in 0..DIRECT_EXTENTS {
            let offset = 192 + i * 20;
            self.extents[i].serialize(&mut buf[offset..offset + 20]);
        }
    }

    /// Serialize inode into exactly 256 bytes.
    pub fn serialize(&self, buf: &mut [u8]) {
        self.serialize_internal(buf);
        let crc = crc32c(&buf[..252]);
        buf[252..256].copy_from_slice(&crc.to_le_bytes());
    }

    /// Deserialize and validate an inode from 256 bytes.
    pub fn deserialize(buf: &[u8]) -> Result<Self> {
        if buf.len() < INODE_SIZE {
            return Err(FsError::Other("Buffer too small for inode".into()));
        }

        let id = u64::from_le_bytes(buf[0..8].try_into().unwrap());
        let file_type = FileType::from_u8(buf[8]);
        let mode = u16::from_le_bytes(buf[9..11].try_into().unwrap());
        let uid = u32::from_le_bytes(buf[12..16].try_into().unwrap());
        let gid = u32::from_le_bytes(buf[16..20].try_into().unwrap());
        let flags = u32::from_le_bytes(buf[20..24].try_into().unwrap());
        let size = u64::from_le_bytes(buf[24..32].try_into().unwrap());
        let allocated_blocks = u64::from_le_bytes(buf[32..40].try_into().unwrap());
        let atime_ns = u64::from_le_bytes(buf[40..48].try_into().unwrap());
        let mtime_ns = u64::from_le_bytes(buf[48..56].try_into().unwrap());
        let ctime_ns = u64::from_le_bytes(buf[56..64].try_into().unwrap());
        let link_count = u32::from_le_bytes(buf[64..68].try_into().unwrap());
        let inline_size = u16::from_le_bytes(buf[68..70].try_into().unwrap());
        let indirect_extent_block = u64::from_le_bytes(buf[72..80].try_into().unwrap());

        let mut inline_data = [0u8; INLINE_DATA_CAPACITY];
        inline_data.copy_from_slice(&buf[80..192]);

        let mut extents = [Extent::empty(); DIRECT_EXTENTS];
        for i in 0..DIRECT_EXTENTS {
            let offset = 192 + i * 20;
            extents[i] = Extent::deserialize(&buf[offset..offset + 20]);
        }

        let crc = u32::from_le_bytes(buf[252..256].try_into().unwrap());
        let computed = crc32c(&buf[..252]);
        if crc != 0 && computed != crc {
            return Err(FsError::CorruptedBlock {
                block_id: id,
                expected: crc,
                actual: computed,
            });
        }

        Ok(Self {
            id,
            file_type,
            mode,
            uid,
            gid,
            flags,
            size,
            allocated_blocks,
            atime_ns,
            mtime_ns,
            ctime_ns,
            link_count,
            inline_size,
            indirect_extent_block,
            inline_data,
            extents,
            crc,
        })
    }

    /// Locate physical block for logical block index in this inode.
    pub fn map_logical_block(&self, logical_block: u32) -> Option<BlockId> {
        for extent in &self.extents {
            if let Some(phys) = extent.map_block(logical_block) {
                return Some(phys);
            }
        }
        None
    }
}
