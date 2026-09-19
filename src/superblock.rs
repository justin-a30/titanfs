//! Superblock layout, serialization, and validation for TitanFS.

use crate::crc::crc32c;
use crate::types::{BLOCK_SIZE, BlockId, FsError, InodeId, Result, ROOT_INODE_ID, TITANFS_MAGIC, TITANFS_VERSION};

/// On-disk filesystem metadata header stored at Block 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Superblock {
    pub magic: u64,
    pub version: u32,
    pub block_size: u32,
    pub total_blocks: u64,
    pub free_blocks: u64,
    pub total_inodes: u64,
    pub free_inodes: u64,
    pub root_inode: InodeId,

    // Journal (Write-Ahead Log) layout
    pub journal_start_block: BlockId,
    pub journal_block_count: u64,

    // Block Allocation Bitmap layout
    pub block_bitmap_start: BlockId,
    pub block_bitmap_blocks: u64,

    // Inode Allocation Bitmap layout
    pub inode_bitmap_start: BlockId,
    pub inode_bitmap_blocks: u64,

    // Inode Table layout
    pub inode_table_start: BlockId,
    pub inode_table_blocks: u64,

    // Data blocks start
    pub data_start_block: BlockId,
    pub data_block_count: u64,

    // Metadata flags and counters
    pub generation: u64,
    pub mount_count: u32,
    pub clean_unmount: u8,
    pub crc: u32,
}

impl Superblock {
    /// Create and initialize a new superblock for a filesystem with given block count.
    pub fn new(total_blocks: u64, total_inodes: u64) -> Self {
        // Layout calculation:
        // Block 0: Superblock (1 block)
        // Blocks 1..1 + J: Journal (e.g. 1024 blocks = 4 MB WAL)
        let journal_start = 1;
        let journal_blocks = if total_blocks >= 16384 { 1024 } else { 128 };

        // Block bitmap: 1 bit per block. 1 block has 4096 * 8 = 32,768 bits.
        let block_bitmap_start = journal_start + journal_blocks;
        let block_bitmap_blocks = (total_blocks + 32767) / 32768;

        // Inode bitmap: 1 bit per inode.
        let inode_bitmap_start = block_bitmap_start + block_bitmap_blocks;
        let inode_bitmap_blocks = (total_inodes + 32767) / 32768;

        // Inode table: 16 inodes per block (256 bytes each).
        let inode_table_start = inode_bitmap_start + inode_bitmap_blocks;
        let inode_table_blocks = (total_inodes + 15) / 16;

        let data_start_block = inode_table_start + inode_table_blocks;
        let data_block_count = if total_blocks > data_start_block {
            total_blocks - data_start_block
        } else {
            0
        };

        let free_blocks = data_block_count;
        let free_inodes = total_inodes.saturating_sub(1); // root inode 1 used

        let mut sb = Self {
            magic: TITANFS_MAGIC,
            version: TITANFS_VERSION,
            block_size: BLOCK_SIZE as u32,
            total_blocks,
            free_blocks,
            total_inodes,
            free_inodes,
            root_inode: ROOT_INODE_ID,
            journal_start_block: journal_start,
            journal_block_count: journal_blocks,
            block_bitmap_start,
            block_bitmap_blocks,
            inode_bitmap_start,
            inode_bitmap_blocks,
            inode_table_start,
            inode_table_blocks,
            data_start_block,
            data_block_count,
            generation: 1,
            mount_count: 0,
            clean_unmount: 1,
            crc: 0,
        };
        sb.crc = sb.compute_crc();
        sb
    }

    /// Compute CRC32C of superblock fields (excluding the checksum field itself).
    pub fn compute_crc(&self) -> u32 {
        let mut buf = [0u8; 256];
        let mut offset = 0;

        macro_rules! write_field {
            ($field:expr) => {
                let bytes = $field.to_le_bytes();
                buf[offset..offset + bytes.len()].copy_from_slice(&bytes);
                offset += bytes.len();
            };
        }

        write_field!(self.magic);
        write_field!(self.version);
        write_field!(self.block_size);
        write_field!(self.total_blocks);
        write_field!(self.free_blocks);
        write_field!(self.total_inodes);
        write_field!(self.free_inodes);
        write_field!(self.root_inode);
        write_field!(self.journal_start_block);
        write_field!(self.journal_block_count);
        write_field!(self.block_bitmap_start);
        write_field!(self.block_bitmap_blocks);
        write_field!(self.inode_bitmap_start);
        write_field!(self.inode_bitmap_blocks);
        write_field!(self.inode_table_start);
        write_field!(self.inode_table_blocks);
        write_field!(self.data_start_block);
        write_field!(self.data_block_count);
        write_field!(self.generation);
        write_field!(self.mount_count);
        buf[offset] = self.clean_unmount;

        crc32c(&buf[..offset + 1])
    }

    /// Serialize superblock into a 4096-byte block.
    pub fn serialize(&self, buf: &mut [u8]) {
        buf.fill(0);
        let mut offset = 0;

        macro_rules! write_field {
            ($field:expr) => {
                let bytes = $field.to_le_bytes();
                buf[offset..offset + bytes.len()].copy_from_slice(&bytes);
                offset += bytes.len();
            };
        }

        write_field!(self.magic);
        write_field!(self.version);
        write_field!(self.block_size);
        write_field!(self.total_blocks);
        write_field!(self.free_blocks);
        write_field!(self.total_inodes);
        write_field!(self.free_inodes);
        write_field!(self.root_inode);
        write_field!(self.journal_start_block);
        write_field!(self.journal_block_count);
        write_field!(self.block_bitmap_start);
        write_field!(self.block_bitmap_blocks);
        write_field!(self.inode_bitmap_start);
        write_field!(self.inode_bitmap_blocks);
        write_field!(self.inode_table_start);
        write_field!(self.inode_table_blocks);
        write_field!(self.data_start_block);
        write_field!(self.data_block_count);
        write_field!(self.generation);
        write_field!(self.mount_count);
        buf[offset] = self.clean_unmount;
        offset += 1;
        write_field!(self.crc);
        let _ = offset;
    }

    /// Deserialize and validate a superblock from a 4096-byte block.
    pub fn deserialize(buf: &[u8]) -> Result<Self> {
        if buf.len() < BLOCK_SIZE {
            return Err(FsError::InvalidSuperblock("Buffer too small for superblock".into()));
        }

        let mut offset = 0;
        macro_rules! read_u64 {
            () => {{
                let val = u64::from_le_bytes(buf[offset..offset + 8].try_into().unwrap());
                offset += 8;
                val
            }};
        }
        macro_rules! read_u32 {
            () => {{
                let val = u32::from_le_bytes(buf[offset..offset + 4].try_into().unwrap());
                offset += 4;
                val
            }};
        }

        let magic = read_u64!();
        if magic != TITANFS_MAGIC {
            return Err(FsError::InvalidSuperblock(format!(
                "Magic mismatch: expected 0x{:016X}, got 0x{:016X}",
                TITANFS_MAGIC, magic
            )));
        }

        let version = read_u32!();
        if version != TITANFS_VERSION {
            return Err(FsError::InvalidSuperblock(format!(
                "Unsupported filesystem version: {}",
                version
            )));
        }

        let block_size = read_u32!();
        if block_size != BLOCK_SIZE as u32 {
            return Err(FsError::InvalidSuperblock(format!(
                "Unsupported block size: {}",
                block_size
            )));
        }

        let total_blocks = read_u64!();
        let free_blocks = read_u64!();
        let total_inodes = read_u64!();
        let free_inodes = read_u64!();
        let root_inode = read_u64!();
        let journal_start_block = read_u64!();
        let journal_block_count = read_u64!();
        let block_bitmap_start = read_u64!();
        let block_bitmap_blocks = read_u64!();
        let inode_bitmap_start = read_u64!();
        let inode_bitmap_blocks = read_u64!();
        let inode_table_start = read_u64!();
        let inode_table_blocks = read_u64!();
        let data_start_block = read_u64!();
        let data_block_count = read_u64!();
        let generation = read_u64!();
        let mount_count = read_u32!();
        let clean_unmount = buf[offset];
        offset += 1;
        let crc = read_u32!();
        let _ = offset;

        let sb = Self {
            magic,
            version,
            block_size,
            total_blocks,
            free_blocks,
            total_inodes,
            free_inodes,
            root_inode,
            journal_start_block,
            journal_block_count,
            block_bitmap_start,
            block_bitmap_blocks,
            inode_bitmap_start,
            inode_bitmap_blocks,
            inode_table_start,
            inode_table_blocks,
            data_start_block,
            data_block_count,
            generation,
            mount_count,
            clean_unmount,
            crc,
        };

        let computed_crc = sb.compute_crc();
        if computed_crc != crc {
            return Err(FsError::InvalidSuperblock(format!(
                "Superblock CRC mismatch (expected 0x{:08X}, computed 0x{:08X})",
                crc, computed_crc
            )));
        }

        Ok(sb)
    }
}
