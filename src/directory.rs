//! Directory entry layout and fast hash-indexed lookup engine.

use crate::crc::crc32c;
use crate::hash::hash64;
use crate::types::{BLOCK_SIZE, FileType, FsError, InodeId, Result, MAX_NAME_LEN};

/// Single directory entry pointing to an inode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub inode_id: InodeId,
    pub file_type: FileType,
    pub name: String,
    pub hash: u64,
}

impl DirEntry {
    pub fn new(inode_id: InodeId, file_type: FileType, name: &str) -> Self {
        Self {
            inode_id,
            file_type,
            name: name.to_string(),
            hash: hash64(name.as_bytes()),
        }
    }

    /// Serialized size of this entry in bytes:
    /// inode_id (8) + file_type (1) + name_len (1) + hash (8) + name_bytes (N)
    pub fn serialized_size(&self) -> usize {
        18 + self.name.len()
    }

    /// Serialize into buffer, returns bytes written.
    pub fn serialize(&self, buf: &mut [u8]) -> usize {
        let name_bytes = self.name.as_bytes();
        let name_len = name_bytes.len().min(MAX_NAME_LEN) as u8;

        buf[0..8].copy_from_slice(&self.inode_id.to_le_bytes());
        buf[8] = self.file_type as u8;
        buf[9] = name_len;
        buf[10..18].copy_from_slice(&self.hash.to_le_bytes());
        buf[18..18 + (name_len as usize)].copy_from_slice(&name_bytes[..name_len as usize]);

        18 + (name_len as usize)
    }

    /// Deserialize entry from buffer, returning (entry, bytes_read).
    pub fn deserialize(buf: &[u8]) -> Option<(Self, usize)> {
        if buf.len() < 18 {
            return None;
        }
        let inode_id = u64::from_le_bytes(buf[0..8].try_into().unwrap());
        if inode_id == 0 {
            return None; // Empty / tombstone
        }
        let file_type = FileType::from_u8(buf[8]);
        let name_len = buf[9] as usize;
        let hash = u64::from_le_bytes(buf[10..18].try_into().unwrap());

        if buf.len() < 18 + name_len {
            return None;
        }
        let name = String::from_utf8_lossy(&buf[18..18 + name_len]).to_string();
        Some((
            Self {
                inode_id,
                file_type,
                name,
                hash,
            },
            18 + name_len,
        ))
    }
}

/// Directory data block manager.
/// Stores packed entries with header: entry_count (u32), used_bytes (u32), crc (u32).
pub struct DirectoryTable;

impl DirectoryTable {
    const HEADER_SIZE: usize = 12; // count (4) + used (4) + crc (4)

    /// Pack a list of directory entries into a 4096-byte block.
    pub fn pack(entries: &[DirEntry], buf: &mut [u8]) -> Result<()> {
        buf.fill(0);
        let count = entries.len() as u32;
        buf[0..4].copy_from_slice(&count.to_le_bytes());

        let mut offset = Self::HEADER_SIZE;
        for entry in entries {
            let needed = entry.serialized_size();
            if offset + needed > BLOCK_SIZE {
                return Err(FsError::Other("Directory block overflow".into()));
            }
            let written = entry.serialize(&mut buf[offset..]);
            offset += written;
        }

        let used = offset as u32;
        buf[4..8].copy_from_slice(&used.to_le_bytes());

        let crc = crc32c(&buf[..offset]);
        buf[8..12].copy_from_slice(&crc.to_le_bytes());
        Ok(())
    }

    /// Pack entries across multiple 4096-byte blocks.
    pub fn pack_multi(entries: &[DirEntry]) -> Vec<[u8; BLOCK_SIZE]> {
        let mut blocks = Vec::new();
        if entries.is_empty() {
            let empty_block = [0u8; BLOCK_SIZE];
            blocks.push(empty_block);
            return blocks;
        }

        let mut cur_block = [0u8; BLOCK_SIZE];
        let mut cur_count = 0u32;
        let mut offset = Self::HEADER_SIZE;

        for entry in entries {
            let needed = entry.serialized_size();
            if offset + needed > BLOCK_SIZE {
                cur_block[0..4].copy_from_slice(&cur_count.to_le_bytes());
                let used = offset as u32;
                cur_block[4..8].copy_from_slice(&used.to_le_bytes());
                let crc = crc32c(&cur_block[..offset]);
                cur_block[8..12].copy_from_slice(&crc.to_le_bytes());
                blocks.push(cur_block);

                cur_block = [0u8; BLOCK_SIZE];
                cur_count = 0;
                offset = Self::HEADER_SIZE;
            }

            let written = entry.serialize(&mut cur_block[offset..]);
            offset += written;
            cur_count += 1;
        }

        if cur_count > 0 || blocks.is_empty() {
            cur_block[0..4].copy_from_slice(&cur_count.to_le_bytes());
            let used = offset as u32;
            cur_block[4..8].copy_from_slice(&used.to_le_bytes());
            let crc = crc32c(&cur_block[..offset]);
            cur_block[8..12].copy_from_slice(&crc.to_le_bytes());
            blocks.push(cur_block);
        }

        blocks
    }

    /// Unpack directory entries from a 4096-byte block.
    pub fn unpack(buf: &[u8]) -> Result<Vec<DirEntry>> {
        if buf.len() < BLOCK_SIZE {
            return Err(FsError::Other("Buffer too small for directory block".into()));
        }

        let count = u32::from_le_bytes(buf[0..4].try_into().unwrap()) as usize;
        let used = u32::from_le_bytes(buf[4..8].try_into().unwrap()) as usize;
        let _crc = u32::from_le_bytes(buf[8..12].try_into().unwrap());

        if used > BLOCK_SIZE || used < Self::HEADER_SIZE {
            return Ok(Vec::new());
        }

        let mut entries = Vec::with_capacity(count);
        let mut offset = Self::HEADER_SIZE;

        while offset < used {
            if let Some((entry, read_len)) = DirEntry::deserialize(&buf[offset..]) {
                entries.push(entry);
                offset += read_len;
            } else {
                break;
            }
        }

        Ok(entries)
    }

    /// Rapidly lookup entry by name using 64-bit hash filter.
    pub fn find_entry<'a>(entries: &'a [DirEntry], name: &str) -> Option<&'a DirEntry> {
        let target_hash = hash64(name.as_bytes());
        for entry in entries {
            if entry.hash == target_hash && entry.name == name {
                return Some(entry);
            }
        }
        None
    }
}
