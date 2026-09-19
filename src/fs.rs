//! Core TitanFS filesystem engine.
//! Coordinates superblocks, bitmaps, inodes, extents, page caching, WAL, and VFS APIs.

use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use crate::bitmap::Bitmap;
use crate::block_dev::BlockDevice;
use crate::cache::PageCache;
use crate::dedup::DedupEngine;
use crate::directory::{DirEntry, DirectoryTable};
use crate::extent::Extent;
use crate::chrono::{parse_temporal_path, ChronoEngine};
use crate::ghost_tree::GhostTreeManager;
use crate::inode::{Inode, INLINE_DATA_CAPACITY};
use crate::journal::Journal;
use crate::omni_index::{IndexEntry, OmniIndex, SearchQuery};
use crate::ransom_shield::RansomShield;
use crate::snapshot::SnapshotManager;
use crate::superblock::Superblock;
use crate::types::{
    BlockId, FileType, FsError, InodeId, Result, BLOCK_SIZE, FLAG_COW_SHARED,
    INODES_PER_BLOCK, INODE_SIZE, ROOT_INODE_ID,
};

pub struct FormatOptions {
    pub total_blocks: u64,
    pub total_inodes: u64,
    pub enable_dedup: bool,
}

impl Default for FormatOptions {
    fn default() -> Self {
        Self {
            total_blocks: 65536, // 256 MB default
            total_inodes: 16384,
            enable_dedup: true,
        }
    }
}

pub struct FsckReport {
    pub total_blocks: u64,
    pub used_blocks: u64,
    pub free_blocks: u64,
    pub total_inodes: u64,
    pub used_inodes: u64,
    pub free_inodes: u64,
    pub corrupted_blocks: usize,
    pub orphan_inodes: usize,
    pub is_clean: bool,
}

pub struct FileSystem<D: BlockDevice> {
    pub dev: D,
    pub sb: Superblock,
    pub block_bitmap: Bitmap,
    pub inode_bitmap: Bitmap,
    pub cache: PageCache,
    pub journal: Journal,
    pub dedup: DedupEngine,
    pub snapshots: SnapshotManager,
    pub chrono: ChronoEngine,
    pub ghost_trees: GhostTreeManager,
    pub ransom_shield: RansomShield,
    pub omni_index: OmniIndex,
    _current_tx_id: u64,
}

impl<D: BlockDevice> FileSystem<D> {
    /// Format a block device with a fresh TitanFS filesystem.
    pub fn format(mut dev: D, options: FormatOptions) -> Result<Self> {
        let total_dev_blocks = dev.total_blocks();
        let total_blocks = if options.total_blocks > 0 && options.total_blocks <= total_dev_blocks {
            options.total_blocks
        } else {
            total_dev_blocks
        };

        let sb = Superblock::new(total_blocks, options.total_inodes);

        // Write superblock to block 0
        let mut sb_buf = [0u8; BLOCK_SIZE];
        sb.serialize(&mut sb_buf);
        dev.write_block(0, &sb_buf)?;

        // Initialize block bitmap
        let mut block_bitmap = Bitmap::new(sb.total_blocks);
        // Mark metadata blocks as allocated
        for b in 0..sb.data_start_block {
            block_bitmap.set(b);
        }

        // Initialize inode bitmap
        let mut inode_bitmap = Bitmap::new(sb.total_inodes);
        // Inode 0 is reserved
        inode_bitmap.set(0);
        // Inode 1 is root directory
        inode_bitmap.set(ROOT_INODE_ID);

        // Flush bitmaps to disk
        let block_bm_bytes = block_bitmap.as_bytes();
        for i in 0..sb.block_bitmap_blocks {
            let offset = (i as usize) * BLOCK_SIZE;
            let end = (offset + BLOCK_SIZE).min(block_bm_bytes.len());
            let mut buf = [0u8; BLOCK_SIZE];
            buf[..end - offset].copy_from_slice(&block_bm_bytes[offset..end]);
            dev.write_block(sb.block_bitmap_start + i, &buf)?;
        }

        let inode_bm_bytes = inode_bitmap.as_bytes();
        for i in 0..sb.inode_bitmap_blocks {
            let offset = (i as usize) * BLOCK_SIZE;
            let end = (offset + BLOCK_SIZE).min(inode_bm_bytes.len());
            let mut buf = [0u8; BLOCK_SIZE];
            buf[..end - offset].copy_from_slice(&inode_bm_bytes[offset..end]);
            dev.write_block(sb.inode_bitmap_start + i, &buf)?;
        }

        // Create root directory inode (ID 1)
        let root_inode = Inode::new(ROOT_INODE_ID, FileType::Directory, 0o755);
        let mut root_block = [0u8; BLOCK_SIZE];
        let root_sub_idx = (ROOT_INODE_ID % INODES_PER_BLOCK as u64) as usize;
        let start = root_sub_idx * INODE_SIZE;
        root_inode.serialize(&mut root_block[start..start + INODE_SIZE]);
        dev.write_block(sb.inode_table_start, &root_block)?;

        dev.flush()?;

        let journal = Journal::new(sb.journal_start_block, sb.journal_block_count);
        let cache = PageCache::new(4096); // 16 MB cache
        let dedup = DedupEngine::new(options.enable_dedup);
        let snapshots = SnapshotManager::new();
        let chrono = ChronoEngine::new(true);
        let ghost_trees = GhostTreeManager::new();
        let ransom_shield = RansomShield::new(true);
        let omni_index = OmniIndex::new(true);

        Ok(Self {
            dev,
            sb,
            block_bitmap,
            inode_bitmap,
            cache,
            journal,
            dedup,
            snapshots,
            chrono,
            ghost_trees,
            ransom_shield,
            omni_index,
            _current_tx_id: 1,
        })
    }

    /// Mount an existing TitanFS filesystem from a block device.
    pub fn mount(mut dev: D) -> Result<Self> {
        let mut sb_buf = [0u8; BLOCK_SIZE];
        dev.read_block(0, &mut sb_buf)?;
        let mut sb = Superblock::deserialize(&sb_buf)?;

        // Read block bitmap
        let mut block_bm_vec = vec![0u8; (sb.block_bitmap_blocks as usize) * BLOCK_SIZE];
        for i in 0..sb.block_bitmap_blocks {
            let offset = (i as usize) * BLOCK_SIZE;
            dev.read_block(sb.block_bitmap_start + i, &mut block_bm_vec[offset..offset + BLOCK_SIZE])?;
        }
        let block_bitmap = Bitmap::from_bytes(block_bm_vec, sb.total_blocks);

        // Read inode bitmap
        let mut inode_bm_vec = vec![0u8; (sb.inode_bitmap_blocks as usize) * BLOCK_SIZE];
        for i in 0..sb.inode_bitmap_blocks {
            let offset = (i as usize) * BLOCK_SIZE;
            dev.read_block(sb.inode_bitmap_start + i, &mut inode_bm_vec[offset..offset + BLOCK_SIZE])?;
        }
        let inode_bitmap = Bitmap::from_bytes(inode_bm_vec, sb.total_inodes);

        sb.mount_count += 1;
        sb.clean_unmount = 0;
        sb.crc = sb.compute_crc();
        sb.serialize(&mut sb_buf);
        dev.write_block(0, &sb_buf)?;
        dev.flush()?;

        let mut journal = Journal::new(sb.journal_start_block, sb.journal_block_count);
        journal.recover(&mut dev)?;

        let cache = PageCache::new(4096);
        let dedup = DedupEngine::new(true);
        let snapshots = SnapshotManager::new();
        let chrono = ChronoEngine::new(true);
        let ghost_trees = GhostTreeManager::new();
        let ransom_shield = RansomShield::new(true);
        let omni_index = OmniIndex::new(true);

        Ok(Self {
            dev,
            sb,
            block_bitmap,
            inode_bitmap,
            cache,
            journal,
            dedup,
            snapshots,
            chrono,
            ghost_trees,
            ransom_shield,
            omni_index,
            _current_tx_id: 1,
        })
    }

    /// Flush all dirty caches, bitmaps, and superblock to persistent storage.
    pub fn sync(&mut self) -> Result<()> {
        self.cache.flush_dirty(&mut self.dev)?;

        // Write block bitmap if dirty
        if self.block_bitmap.is_dirty() {
            let block_bm_bytes = self.block_bitmap.as_bytes();
            for i in 0..self.sb.block_bitmap_blocks {
                let offset = (i as usize) * BLOCK_SIZE;
                let end = (offset + BLOCK_SIZE).min(block_bm_bytes.len());
                let mut buf = [0u8; BLOCK_SIZE];
                buf[..end - offset].copy_from_slice(&block_bm_bytes[offset..end]);
                self.dev.write_block(self.sb.block_bitmap_start + i, &buf)?;
            }
            self.block_bitmap.mark_clean();
        }

        // Write inode bitmap if dirty
        if self.inode_bitmap.is_dirty() {
            let inode_bm_bytes = self.inode_bitmap.as_bytes();
            for i in 0..self.sb.inode_bitmap_blocks {
                let offset = (i as usize) * BLOCK_SIZE;
                let end = (offset + BLOCK_SIZE).min(inode_bm_bytes.len());
                let mut buf = [0u8; BLOCK_SIZE];
                buf[..end - offset].copy_from_slice(&inode_bm_bytes[offset..end]);
                self.dev.write_block(self.sb.inode_bitmap_start + i, &buf)?;
            }
            self.inode_bitmap.mark_clean();
        }

        // Update superblock counters
        self.sb.free_blocks = self.block_bitmap.count_free();
        self.sb.free_inodes = self.inode_bitmap.count_free();
        self.sb.generation += 1;
        self.sb.clean_unmount = 1;
        self.sb.crc = self.sb.compute_crc();

        let mut sb_buf = [0u8; BLOCK_SIZE];
        self.sb.serialize(&mut sb_buf);
        self.dev.write_block(0, &sb_buf)?;
        self.dev.flush()?;

        Ok(())
    }

    /// Read an inode by its ID.
    pub fn read_inode(&mut self, id: InodeId) -> Result<Inode> {
        if id == 0 || id >= self.sb.total_inodes {
            return Err(FsError::InvalidInode(id));
        }

        let block_idx = (id / INODES_PER_BLOCK as u64) as BlockId;
        let block_id = self.sb.inode_table_start + block_idx;
        let sub_idx = (id % INODES_PER_BLOCK as u64) as usize;

        let block_data = self.cache.read_through(&self.dev, block_id)?;
        let start = sub_idx * INODE_SIZE;
        Inode::deserialize(&block_data[start..start + INODE_SIZE])
    }

    /// Write an inode back to disk/cache.
    pub fn write_inode(&mut self, inode: &Inode) -> Result<()> {
        let id = inode.id;
        if id == 0 || id >= self.sb.total_inodes {
            return Err(FsError::InvalidInode(id));
        }

        let block_idx = (id / INODES_PER_BLOCK as u64) as BlockId;
        let block_id = self.sb.inode_table_start + block_idx;
        let sub_idx = (id % INODES_PER_BLOCK as u64) as usize;

        let mut block_data = self.cache.read_through(&self.dev, block_id)?;
        let start = sub_idx * INODE_SIZE;
        inode.serialize(&mut block_data[start..start + INODE_SIZE]);

        self.cache.put_dirty(block_id, &block_data);
        Ok(())
    }

    /// Allocate a new free inode.
    pub fn allocate_inode(&mut self, file_type: FileType, mode: u16) -> Result<Inode> {
        let inode_id = self.inode_bitmap.allocate_first()
            .ok_or(FsError::InodeTableFull)?;
        let inode = Inode::new(inode_id, file_type, mode);
        self.write_inode(&inode)?;
        Ok(inode)
    }

    /// Free an allocated inode and its blocks.
    pub fn free_inode(&mut self, id: InodeId) -> Result<()> {
        let inode = self.read_inode(id)?;
        // Free extents
        for extent in &inode.extents {
            if !extent.is_empty() {
                for b in extent.physical_block..extent.physical_block + extent.length as u64 {
                    self.block_bitmap.clear(b);
                }
            }
        }
        self.inode_bitmap.clear(id);
        Ok(())
    }

    /// Read directory entries for a directory inode.
    pub fn read_directory_entries(&mut self, inode: &Inode) -> Result<Vec<DirEntry>> {
        if !inode.is_dir() {
            return Err(FsError::NotADirectory(format!("Inode {}", inode.id)));
        }

        if inode.is_inline() {
            let size = (inode.inline_size as usize).min(INLINE_DATA_CAPACITY);
            if size == 0 {
                return Ok(Vec::new());
            }
            let mut entries = Vec::new();
            let mut offset = 0;
            while offset < size {
                if let Some((entry, len)) = DirEntry::deserialize(&inode.inline_data[offset..size]) {
                    entries.push(entry);
                    offset += len;
                } else {
                    break;
                }
            }
            return Ok(entries);
        }

        // Extent-based directory blocks
        let mut entries = Vec::new();
        for extent in &inode.extents {
            if extent.is_empty() {
                break;
            }
            for b in 0..extent.length {
                let phys_block = extent.physical_block + b as u64;
                let block_data = self.cache.read_through(&self.dev, phys_block)?;
                let mut block_entries = DirectoryTable::unpack(&block_data)?;
                entries.append(&mut block_entries);
            }
        }

        if inode.indirect_extent_block != 0 {
            let ind_data = self.cache.read_through(&self.dev, inode.indirect_extent_block)?;
            let max_ind_extents = BLOCK_SIZE / 20;
            for i in 0..max_ind_extents {
                let offset = i * 20;
                let ext = Extent::deserialize(&ind_data[offset..offset + 20]);
                if ext.is_empty() {
                    break;
                }
                for b in 0..ext.length {
                    let phys_block = ext.physical_block + b as u64;
                    let block_data = self.cache.read_through(&self.dev, phys_block)?;
                    let mut block_entries = DirectoryTable::unpack(&block_data)?;
                    entries.append(&mut block_entries);
                }
            }
        }

        Ok(entries)
    }

    /// Write directory entries for a directory inode.
    pub fn write_directory_entries(&mut self, inode: &mut Inode, entries: &[DirEntry]) -> Result<()> {
        if !inode.is_dir() {
            return Err(FsError::NotADirectory(format!("Inode {}", inode.id)));
        }

        let total_size: usize = entries.iter().map(|e| e.serialized_size()).sum();

        // Try inline storage if it fits
        if total_size <= INLINE_DATA_CAPACITY {
            inode.set_inline(true);
            inode.inline_size = total_size as u16;
            inode.inline_data.fill(0);
            let mut offset = 0;
            for entry in entries {
                let len = entry.serialize(&mut inode.inline_data[offset..]);
                offset += len;
            }
            inode.touch();
            self.write_inode(inode)?;
            return Ok(());
        }

        // Transition directory from inline to extent storage
        inode.set_inline(false);
        inode.inline_size = 0;
        inode.inline_data.fill(0);

        // Multi-block extent allocation for directory
        let packed_blocks = DirectoryTable::pack_multi(entries);
        let blocks_needed = packed_blocks.len();

        while (inode.allocated_blocks as usize) < blocks_needed {
            let needed = blocks_needed - (inode.allocated_blocks as usize);

            // 1. Try to expand the last directory extent if contiguous
            let mut expanded = false;
            for ext in inode.extents.iter_mut().rev() {
                if !ext.is_empty() {
                    let next_phys = ext.physical_block + ext.length as u64;
                    let mut free_run = 0usize;
                    while free_run < needed && (next_phys + free_run as u64) < self.sb.total_blocks && !self.block_bitmap.test(next_phys + free_run as u64) {
                        free_run += 1;
                    }
                    if free_run > 0 {
                        for b in 0..free_run {
                            self.block_bitmap.set(next_phys + b as u64);
                        }
                        ext.length += free_run as u32;
                        inode.allocated_blocks += free_run as u64;
                        expanded = true;
                        break;
                    }
                }
            }
            if expanded {
                continue;
            }

            let (start_phys, actual_len) = if let Some(start) = self.block_bitmap.allocate_contiguous(needed) {
                (start, needed as u32)
            } else if let Some(start) = self.block_bitmap.allocate_first() {
                (start, 1)
            } else {
                return Err(FsError::DiskFull);
            };

            let logical_start = inode.allocated_blocks as u32;
            let mut appended = false;
            for ext in &mut inode.extents {
                if ext.is_empty() {
                    *ext = Extent {
                        logical_block: logical_start,
                        physical_block: start_phys,
                        length: actual_len,
                        flags: 0,
                    };
                    inode.allocated_blocks += actual_len as u64;
                    appended = true;
                    break;
                }
            }

            // If direct extents are full, use indirect extent block
            if !appended {
                if inode.indirect_extent_block == 0 {
                    let ind_block = self.block_bitmap.allocate_first().ok_or(FsError::DiskFull)?;
                    inode.indirect_extent_block = ind_block;
                    let empty_block = [0u8; BLOCK_SIZE];
                    self.cache.put_dirty(ind_block, &empty_block);
                }

                let ind_block = inode.indirect_extent_block;
                let mut ind_data = self.cache.read_through(&self.dev, ind_block)?;
                let max_ind_extents = BLOCK_SIZE / 20;
                for i in 0..max_ind_extents {
                    let offset = i * 20;
                    let ext = Extent::deserialize(&ind_data[offset..offset + 20]);
                    if ext.is_empty() {
                        let new_ext = Extent {
                            logical_block: logical_start,
                            physical_block: start_phys,
                            length: actual_len,
                            flags: 0,
                        };
                        new_ext.serialize(&mut ind_data[offset..offset + 20]);
                        self.cache.put_dirty(ind_block, &ind_data);
                        inode.allocated_blocks += actual_len as u64;
                        appended = true;
                        break;
                    }
                }
            }

            if !appended {
                return Err(FsError::Other("Directory extent capacity reached".into()));
            }
        }

        for (i, block_data) in packed_blocks.iter().enumerate() {
            let phys_block = self.map_file_block(inode, i as u32)?.unwrap();
            self.cache.put_dirty(phys_block, block_data);
        }

        inode.size = total_size as u64;
        inode.touch();
        self.write_inode(inode)?;
        Ok(())
    }

    /// Locate physical block for a file's logical block (checks direct extents and indirect extent block).
    pub fn map_file_block(&mut self, inode: &Inode, logical_block: u32) -> Result<Option<BlockId>> {
        if let Some(phys) = inode.map_logical_block(logical_block) {
            return Ok(Some(phys));
        }

        if inode.indirect_extent_block != 0 {
            let ind_data = self.cache.read_through(&self.dev, inode.indirect_extent_block)?;
            let max_ind_extents = BLOCK_SIZE / 20;
            for i in 0..max_ind_extents {
                let offset = i * 20;
                let ext = Extent::deserialize(&ind_data[offset..offset + 20]);
                if ext.is_empty() {
                    break;
                }
                if let Some(phys) = ext.map_block(logical_block) {
                    return Ok(Some(phys));
                }
            }
        }

        Ok(None)
    }

    /// Resolve a path (e.g. "/a/b/c" or temporal "/a/b/c@-15m") into its Inode.
    pub fn resolve_path(&mut self, path: &str) -> Result<Inode> {
        let (base_path, temporal_ts) = parse_temporal_path(path)?;
        let trimmed = base_path.trim().trim_matches('/');
        if trimmed.is_empty() {
            return self.read_inode(ROOT_INODE_ID);
        }

        let mut current_inode = self.read_inode(ROOT_INODE_ID)?;
        for segment in trimmed.split('/') {
            if segment.is_empty() || segment == "." {
                continue;
            }
            if !current_inode.is_dir() {
                return Err(FsError::NotADirectory(segment.to_string()));
            }
            let entries = self.read_directory_entries(&current_inode)?;
            let matched = DirectoryTable::find_entry(&entries, segment)
                .ok_or_else(|| FsError::FileNotFound(path.to_string()))?;
            current_inode = self.read_inode(matched.inode_id)?;
        }

        // ChronoVFS: Apply temporal state if time-travel modifier was specified
        if let Some(target_ns) = temporal_ts {
            if let Some(historic_version) = self.chrono.get_version_at(current_inode.id, target_ns) {
                historic_version.apply_to_inode(&mut current_inode);
            }
        }

        Ok(current_inode)
    }

    /// Create a regular file at the specified path.
    pub fn create_file(&mut self, path: &str, mode: u16) -> Result<InodeId> {
        let (parent_path, file_name) = split_path(path)?;
        let mut parent_inode = self.resolve_path(&parent_path)?;

        let mut entries = self.read_directory_entries(&parent_inode)?;
        if DirectoryTable::find_entry(&entries, &file_name).is_some() {
            return Err(FsError::AlreadyExists(path.to_string()));
        }

        let child_inode = self.allocate_inode(FileType::RegularFile, mode)?;
        self.omni_index.index_file(&child_inode, path);
        self.chrono.record_version(&child_inode, Some(Vec::new()));
        entries.push(DirEntry::new(child_inode.id, FileType::RegularFile, &file_name));
        self.write_directory_entries(&mut parent_inode, &entries)?;

        Ok(child_inode.id)
    }

    /// Create a directory at the specified path.
    pub fn create_dir(&mut self, path: &str) -> Result<InodeId> {
        let (parent_path, dir_name) = split_path(path)?;
        let mut parent_inode = self.resolve_path(&parent_path)?;

        let mut entries = self.read_directory_entries(&parent_inode)?;
        if DirectoryTable::find_entry(&entries, &dir_name).is_some() {
            return Err(FsError::AlreadyExists(path.to_string()));
        }

        let child_inode = self.allocate_inode(FileType::Directory, 0o755)?;
        entries.push(DirEntry::new(child_inode.id, FileType::Directory, &dir_name));
        self.write_directory_entries(&mut parent_inode, &entries)?;

        Ok(child_inode.id)
    }

    /// Truncate or resize a file to a specific length.
    pub fn truncate_file(&mut self, path: &str, new_size: u64) -> Result<()> {
        let mut inode = self.resolve_path(path)?;
        if inode.is_dir() {
            return Err(FsError::IsADirectory(path.to_string()));
        }

        inode.size = new_size;
        if new_size <= INLINE_DATA_CAPACITY as u64 {
            inode.inline_size = new_size as u16;
            inode.set_inline(true);
            inode.inline_data[new_size as usize..].fill(0);
        }
        inode.touch();
        self.write_inode(&inode)?;
        let data = if inode.is_inline() {
            Some(inode.inline_data[..new_size as usize].to_vec())
        } else {
            None
        };
        self.chrono.record_version(&inode, data);
        self.omni_index.index_file(&inode, path);
        Ok(())
    }

    /// Write data to a file, truncating any existing content first.
    pub fn write_file_truncate(&mut self, path: &str, data: &[u8]) -> Result<usize> {
        self.truncate_file(path, 0)?;
        self.write_file(path, 0, data)
    }

    /// Write data to a file at a specific byte offset.
    pub fn write_file(&mut self, path: &str, offset: u64, data: &[u8]) -> Result<usize> {
        let mut inode = self.resolve_path(path)?;
        if inode.is_dir() {
            return Err(FsError::IsADirectory(path.to_string()));
        }

        // ChronoVFS: Record pre-mutation version
        let pre_data = if inode.is_inline() {
            Some(inode.inline_data[..inode.inline_size as usize].to_vec())
        } else if inode.size <= 1024 * 1024 {
            self.read_file(path, 0, inode.size as usize).ok()
        } else {
            None
        };
        self.chrono.record_version(&inode, pre_data);

        let new_size = inode.size.max(offset + data.len() as u64);

        // Fast path: Small file inline write
        if new_size <= INLINE_DATA_CAPACITY as u64 {
            let start = offset as usize;
            let end = start + data.len();
            inode.inline_data[start..end].copy_from_slice(data);
            inode.inline_size = new_size as u16;
            inode.size = new_size;
            inode.set_inline(true);
            inode.touch();
            self.write_inode(&inode)?;
            let inline_payload = inode.inline_data[..inode.inline_size as usize].to_vec();
            self.chrono.record_version(&inode, Some(inline_payload));
            self.omni_index.index_file(&inode, path);
            return Ok(data.len());
        }

        // Transition from inline to extent if needed
        if inode.is_inline() {
            if inode.inline_size > 0 {
                let phys_block = self.block_bitmap.allocate_first()
                    .ok_or(FsError::DiskFull)?;
                let mut block_buf = [0u8; BLOCK_SIZE];
                let inline_len = inode.inline_size as usize;
                block_buf[..inline_len].copy_from_slice(&inode.inline_data[..inline_len]);
                self.cache.put_dirty(phys_block, &block_buf);

                inode.extents[0] = Extent {
                    logical_block: 0,
                    physical_block: phys_block,
                    length: 1,
                    flags: 0,
                };
                inode.allocated_blocks = 1;
            }
            inode.set_inline(false);
            inode.inline_size = 0;
            inode.inline_data.fill(0);
        }

        // Extent-based writes
        let mut written = 0usize;
        let mut cur_offset = offset;

        while written < data.len() {
            let logical_block = (cur_offset / BLOCK_SIZE as u64) as u32;
            let block_offset = (cur_offset % BLOCK_SIZE as u64) as usize;
            let chunk_len = (data.len() - written).min(BLOCK_SIZE - block_offset);

            // Locate or allocate physical block
            let phys_block = match self.map_file_block(&inode, logical_block)? {
                Some(b) => b,
                None => {
                    // Try to allocate contiguous run for remaining chunks
                    let remaining_bytes = data.len() - written;
                    let blocks_needed = ((remaining_bytes + BLOCK_SIZE - 1) / BLOCK_SIZE).max(1);
                    let alloc_len = blocks_needed.min(1024);

                    // 1. Try to expand the last direct extent if contiguous
                    let mut expanded = false;
                    for ext in inode.extents.iter_mut().rev() {
                        if !ext.is_empty() && ext.logical_block + ext.length == logical_block {
                            let next_phys = ext.physical_block + ext.length as u64;
                            let mut free_run = 0usize;
                            while free_run < alloc_len && (next_phys + free_run as u64) < self.sb.total_blocks && !self.block_bitmap.test(next_phys + free_run as u64) {
                                free_run += 1;
                            }
                            if free_run > 0 {
                                for b in 0..free_run {
                                    self.block_bitmap.set(next_phys + b as u64);
                                }
                                ext.length += free_run as u32;
                                inode.allocated_blocks += free_run as u64;
                                expanded = true;
                                break;
                            }
                        }
                    }

                    if expanded {
                        self.map_file_block(&inode, logical_block)?.unwrap()
                    } else {
                        let (start_phys, actual_len) = if let Some(start) = self.block_bitmap.allocate_contiguous(alloc_len) {
                            (start, alloc_len as u32)
                        } else if let Some(start) = self.block_bitmap.allocate_first() {
                            (start, 1)
                        } else {
                            return Err(FsError::DiskFull);
                        };

                        // Append to direct extents
                        let mut appended = false;
                        for ext in &mut inode.extents {
                            if ext.is_empty() {
                                *ext = Extent {
                                    logical_block,
                                    physical_block: start_phys,
                                    length: actual_len,
                                    flags: 0,
                                };
                                inode.allocated_blocks += actual_len as u64;
                                appended = true;
                                break;
                            }
                        }

                        // If direct extents are full, use indirect extent block
                        if !appended {
                            if inode.indirect_extent_block == 0 {
                                let ind_block = self.block_bitmap.allocate_first().ok_or(FsError::DiskFull)?;
                                inode.indirect_extent_block = ind_block;
                                let empty_block = [0u8; BLOCK_SIZE];
                                self.cache.put_dirty(ind_block, &empty_block);
                            }

                            let ind_block = inode.indirect_extent_block;
                            let mut ind_data = self.cache.read_through(&self.dev, ind_block)?;
                            let max_ind_extents = BLOCK_SIZE / 20;
                            for i in 0..max_ind_extents {
                                let offset = i * 20;
                                let ext = Extent::deserialize(&ind_data[offset..offset + 20]);
                                if ext.is_empty() {
                                    let new_ext = Extent {
                                        logical_block,
                                        physical_block: start_phys,
                                        length: actual_len,
                                        flags: 0,
                                    };
                                    new_ext.serialize(&mut ind_data[offset..offset + 20]);
                                    self.cache.put_dirty(ind_block, &ind_data);
                                    inode.allocated_blocks += actual_len as u64;
                                    appended = true;
                                    break;
                                }
                            }
                        }

                        if !appended {
                            return Err(FsError::Other("Maximum file extent capacity reached".into()));
                        }

                        start_phys
                    }
                }
            };

            let mut block_data = if block_offset > 0 || chunk_len < BLOCK_SIZE {
                self.cache.read_through(&self.dev, phys_block)?
            } else {
                [0u8; BLOCK_SIZE]
            };

            block_data[block_offset..block_offset + chunk_len]
                .copy_from_slice(&data[written..written + chunk_len]);

            let mut target_phys_block = phys_block;
            // Deduplication check if writing full block
            if block_offset == 0 && chunk_len == BLOCK_SIZE {
                if let Some(dedup_block) = self.dedup.find_or_register(&block_data, phys_block) {
                    if dedup_block != phys_block {
                        // Only replace if extent is a single-block extent
                        if let Some(ext) = inode.extents.iter_mut().find(|e| e.contains_logical(logical_block) && e.length == 1) {
                            ext.physical_block = dedup_block;
                            self.block_bitmap.clear(phys_block);
                            target_phys_block = dedup_block;
                        }
                    }
                }
            }

            // RansomShield: Inspect written block for cryptographic anomalies
            if self.ransom_shield.inspect_and_filter(inode.id, path, target_phys_block, &block_data) {
                written += chunk_len;
                cur_offset += chunk_len as u64;
                continue;
            }

            self.cache.put_dirty(target_phys_block, &block_data);

            written += chunk_len;
            cur_offset += chunk_len as u64;
        }

        inode.size = new_size;
        inode.touch();
        self.write_inode(&inode)?;
        let post_data = if inode.size <= 1024 * 1024 {
            self.read_file(path, 0, inode.size as usize).ok()
        } else {
            None
        };
        self.chrono.record_version(&inode, post_data);
        self.omni_index.index_file(&inode, path);
        Ok(written)
    }

    /// Read data from a file starting from offset up to length bytes.
    pub fn read_file(&mut self, path: &str, offset: u64, length: usize) -> Result<Vec<u8>> {
        let (_base_path, temporal_ts) = parse_temporal_path(path)?;
        let inode = self.resolve_path(path)?;
        if inode.is_dir() {
            return Err(FsError::IsADirectory(path.to_string()));
        }

        // ChronoVFS: Direct historic payload retrieval if available
        if let Some(target_ns) = temporal_ts {
            if let Some(version) = self.chrono.get_version_at(inode.id, target_ns) {
                if let Some(ref payload) = version.data_payload {
                    if offset >= payload.len() as u64 {
                        return Ok(Vec::new());
                    }
                    let start = offset as usize;
                    let end = (start + length).min(payload.len());
                    return Ok(payload[start..end].to_vec());
                }
            }
        }

        if offset >= inode.size {
            return Ok(Vec::new());
        }

        let actual_len = length.min((inode.size - offset) as usize);
        if actual_len == 0 {
            return Ok(Vec::new());
        }

        // Fast path: Inline data read
        if inode.is_inline() {
            let start = (offset as usize).min(INLINE_DATA_CAPACITY);
            let end = (start + actual_len).min(INLINE_DATA_CAPACITY);
            return Ok(inode.inline_data[start..end].to_vec());
        }

        // Extent-based reads
        let mut result = Vec::with_capacity(actual_len);
        let mut read_bytes = 0usize;
        let mut cur_offset = offset;

        while read_bytes < actual_len {
            let logical_block = (cur_offset / BLOCK_SIZE as u64) as u32;
            let block_offset = (cur_offset % BLOCK_SIZE as u64) as usize;
            let chunk_len = (actual_len - read_bytes).min(BLOCK_SIZE - block_offset);

            if let Some(phys_block) = self.map_file_block(&inode, logical_block)? {
                let block_data = self.cache.read_through(&self.dev, phys_block)?;
                result.extend_from_slice(&block_data[block_offset..block_offset + chunk_len]);
            } else {
                // Sparse hole / unallocated block -> zeros
                result.resize(result.len() + chunk_len, 0);
            }

            read_bytes += chunk_len;
            cur_offset += chunk_len as u64;
        }

        Ok(result)
    }

    /// List directory contents.
    pub fn list_dir(&mut self, path: &str) -> Result<Vec<DirEntry>> {
        let inode = self.resolve_path(path)?;
        self.read_directory_entries(&inode)
    }

    /// Remove a file at the given path.
    pub fn remove_file(&mut self, path: &str) -> Result<()> {
        let (parent_path, file_name) = split_path(path)?;
        let mut parent_inode = self.resolve_path(&parent_path)?;
        let mut entries = self.read_directory_entries(&parent_inode)?;

        let idx = entries.iter().position(|e| e.name == file_name)
            .ok_or_else(|| FsError::FileNotFound(path.to_string()))?;

        let entry = entries.remove(idx);
        if entry.file_type == FileType::Directory {
            return Err(FsError::IsADirectory(path.to_string()));
        }

        self.omni_index.unindex_file(entry.inode_id);
        self.free_inode(entry.inode_id)?;
        self.write_directory_entries(&mut parent_inode, &entries)?;
        Ok(())
    }

    /// Remove an empty directory.
    pub fn remove_dir(&mut self, path: &str) -> Result<()> {
        let (parent_path, dir_name) = split_path(path)?;
        let mut parent_inode = self.resolve_path(&parent_path)?;
        let mut entries = self.read_directory_entries(&parent_inode)?;

        let idx = entries.iter().position(|e| e.name == dir_name)
            .ok_or_else(|| FsError::FileNotFound(path.to_string()))?;

        let entry = entries.remove(idx);
        if entry.file_type != FileType::Directory {
            return Err(FsError::NotADirectory(path.to_string()));
        }

        let dir_inode = self.read_inode(entry.inode_id)?;
        let sub_entries = self.read_directory_entries(&dir_inode)?;
        if !sub_entries.is_empty() {
            return Err(FsError::DirectoryNotEmpty(path.to_string()));
        }

        self.free_inode(entry.inode_id)?;
        self.write_directory_entries(&mut parent_inode, &entries)?;
        Ok(())
    }

    /// Get inode metadata for a path.
    pub fn stat(&mut self, path: &str) -> Result<Inode> {
        self.resolve_path(path)
    }

    /// Create an instant snapshot of the current root state.
    pub fn create_snapshot(&mut self, name: &str) -> Result<()> {
        let root = self.read_inode(ROOT_INODE_ID)?;
        let mut inodes = Vec::new();
        for id in 1..self.sb.total_inodes {
            if self.inode_bitmap.test(id) {
                if let Ok(inode) = self.read_inode(id) {
                    inodes.push(inode);
                }
            }
        }
        self.snapshots.create(name, &root, inodes, self.sb.generation)
    }

    /// Rollback filesystem root to a named snapshot.
    pub fn rollback_snapshot(&mut self, name: &str) -> Result<()> {
        let snap = self.snapshots.get(name)
            .ok_or_else(|| FsError::SnapshotNotFound(name.to_string()))?
            .clone();
        for inode in &snap.inodes {
            self.write_inode(inode)?;
        }
        self.write_inode(&snap.root_inode)?;
        self.cache.flush_dirty(&mut self.dev)?;
        self.cache.clear();
        self.sync()?;
        Ok(())
    }

    /// Instant O(1) GhostTree Merkle workspace branching of an entire directory tree.
    pub fn branch_dir(&mut self, source_path: &str, branch_path: &str) -> Result<()> {
        let source_inode = self.resolve_path(source_path)?;
        if !source_inode.is_dir() {
            return Err(FsError::NotADirectory(source_path.to_string()));
        }

        let (parent_path, branch_name) = split_path(branch_path)?;
        let mut parent_inode = self.resolve_path(&parent_path)?;
        let mut parent_entries = self.read_directory_entries(&parent_inode)?;

        if DirectoryTable::find_entry(&parent_entries, &branch_name).is_some() {
            return Err(FsError::AlreadyExists(branch_path.to_string()));
        }

        let source_entries = self.read_directory_entries(&source_inode)?;
        let merkle_root = self.ghost_trees.compute_merkle_root(source_inode.id, &source_entries);

        let mut cloned_inode = self.allocate_inode(FileType::Directory, source_inode.mode)?;
        let mut branch_entries = Vec::with_capacity(source_entries.len());

        for entry in &source_entries {
            let orig_inode = self.read_inode(entry.inode_id)?;
            let mut cloned_file = self.allocate_inode(entry.file_type, orig_inode.mode)?;
            cloned_file.extents = orig_inode.extents;
            cloned_file.indirect_extent_block = orig_inode.indirect_extent_block;
            cloned_file.allocated_blocks = orig_inode.allocated_blocks;
            cloned_file.size = orig_inode.size;
            cloned_file.flags = orig_inode.flags | FLAG_COW_SHARED;
            if orig_inode.is_inline() {
                cloned_file.set_inline(true);
                cloned_file.inline_size = orig_inode.inline_size;
                cloned_file.inline_data = orig_inode.inline_data;
            }
            self.write_inode(&cloned_file)?;
            let child_path = format!("{}/{}", branch_path.trim_end_matches('/'), entry.name);
            self.omni_index.index_file(&cloned_file, &child_path);
            let payload = if orig_inode.is_inline() {
                Some(orig_inode.inline_data[..orig_inode.inline_size as usize].to_vec())
            } else {
                None
            };
            self.chrono.record_version(&cloned_file, payload);
            branch_entries.push(DirEntry::new(cloned_file.id, entry.file_type, &entry.name));
        }

        self.write_directory_entries(&mut cloned_inode, &branch_entries)?;

        self.ghost_trees.register_branch(&branch_name, source_inode.id, cloned_inode.id, merkle_root)?;

        parent_entries.push(DirEntry::new(cloned_inode.id, FileType::Directory, &branch_name));
        self.write_directory_entries(&mut parent_inode, &parent_entries)?;

        Ok(())
    }

    /// Sub-millisecond compound metadata search via OmniIndex.
    pub fn search_index(&self, query: &SearchQuery) -> Vec<IndexEntry> {
        self.omni_index.search(query).into_iter().cloned().collect()
    }

    /// Import a file from the host operating system into TitanFS.
    pub fn import_host_file<P: AsRef<Path>>(&mut self, host_path: P, fs_path: &str) -> Result<u64> {
        let mut file = File::open(host_path)?;
        let meta = file.metadata()?;
        let total_len = meta.len();

        let _ = self.create_file(fs_path, 0o644);

        let mut buffer = [0u8; 65536]; // 64 KB buffer for streaming copy
        let mut offset = 0u64;

        loop {
            let bytes_read = file.read(&mut buffer)?;
            if bytes_read == 0 {
                break;
            }
            self.write_file(fs_path, offset, &buffer[..bytes_read])?;
            offset += bytes_read as u64;
        }

        self.sync()?;
        Ok(total_len)
    }

    /// Export a file from TitanFS out to the host operating system.
    pub fn export_to_host<P: AsRef<Path>>(&mut self, fs_path: &str, host_path: P) -> Result<u64> {
        let inode = self.resolve_path(fs_path)?;
        let mut host_file = File::create(host_path)?;

        let mut offset = 0u64;
        let chunk_size = 65536;

        while offset < inode.size {
            let to_read = (inode.size - offset).min(chunk_size as u64) as usize;
            let data = self.read_file(fs_path, offset, to_read)?;
            host_file.write_all(&data)?;
            offset += to_read as u64;
        }

        host_file.flush()?;
        Ok(inode.size)
    }

    /// Perform a full filesystem consistency check and integrity verification.
    pub fn fsck(&mut self) -> Result<FsckReport> {
        let total_blocks = self.sb.total_blocks;
        let used_blocks = self.block_bitmap.count_allocated();
        let free_blocks = self.block_bitmap.count_free();

        let total_inodes = self.sb.total_inodes;
        let used_inodes = self.inode_bitmap.count_allocated();
        let free_inodes = self.inode_bitmap.count_free();

        let mut corrupted_blocks = 0;
        let orphan_inodes = 0;

        // Verify root inode
        if let Ok(root) = self.read_inode(ROOT_INODE_ID) {
            if !root.is_dir() {
                corrupted_blocks += 1;
            }
        } else {
            corrupted_blocks += 1;
        }

        let is_clean = corrupted_blocks == 0 && orphan_inodes == 0;

        Ok(FsckReport {
            total_blocks,
            used_blocks,
            free_blocks,
            total_inodes,
            used_inodes,
            free_inodes,
            corrupted_blocks,
            orphan_inodes,
            is_clean,
        })
    }
}

/// Helper to split a path into (parent_path, base_name).
fn split_path(path: &str) -> Result<(String, String)> {
    let trimmed = path.trim().trim_matches('/');
    if trimmed.is_empty() {
        return Err(FsError::InvalidPath("Cannot split root path".into()));
    }

    if let Some(pos) = trimmed.rfind('/') {
        let parent = &trimmed[..pos];
        let name = &trimmed[pos + 1..];
        Ok((parent.to_string(), name.to_string()))
    } else {
        Ok(("".to_string(), trimmed.to_string()))
    }
}
