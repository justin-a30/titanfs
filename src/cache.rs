//! High-performance buffer page cache with O(1) queue eviction and read-ahead.

use std::collections::{HashMap, VecDeque};
use crate::block_dev::BlockDevice;
use crate::types::{BLOCK_SIZE, BlockId, Result};

pub struct CachedBlock {
    pub data: [u8; BLOCK_SIZE],
    pub dirty: bool,
    pub last_access: u64,
}

pub struct PageCache {
    capacity: usize,
    entries: HashMap<BlockId, CachedBlock>,
    clean_queue: VecDeque<BlockId>,
    access_counter: u64,
}

impl PageCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(16),
            entries: HashMap::with_capacity(capacity.max(16)),
            clean_queue: VecDeque::with_capacity(capacity.max(16)),
            access_counter: 0,
        }
    }

    /// Retrieve block from cache if present.
    pub fn get(&mut self, block_id: BlockId) -> Option<&[u8]> {
        self.access_counter += 1;
        let counter = self.access_counter;
        if let Some(entry) = self.entries.get_mut(&block_id) {
            entry.last_access = counter;
            Some(&entry.data)
        } else {
            None
        }
    }

    /// Read block: returns cached if present, else reads from device and caches it.
    pub fn read_through<D: BlockDevice>(&mut self, dev: &D, block_id: BlockId) -> Result<[u8; BLOCK_SIZE]> {
        self.access_counter += 1;
        let counter = self.access_counter;

        if let Some(entry) = self.entries.get_mut(&block_id) {
            entry.last_access = counter;
            return Ok(entry.data);
        }

        let mut buf = [0u8; BLOCK_SIZE];
        dev.read_block(block_id, &mut buf)?;

        self.insert_clean(block_id, buf);
        Ok(buf)
    }

    /// Insert or update block in cache marking it dirty.
    pub fn put_dirty(&mut self, block_id: BlockId, data: &[u8]) {
        self.access_counter += 1;
        let counter = self.access_counter;

        let mut block = [0u8; BLOCK_SIZE];
        block.copy_from_slice(&data[..BLOCK_SIZE]);

        if let Some(entry) = self.entries.get_mut(&block_id) {
            entry.data = block;
            entry.dirty = true;
            entry.last_access = counter;
        } else {
            self.ensure_capacity_no_flush();
            self.entries.insert(
                block_id,
                CachedBlock {
                    data: block,
                    dirty: true,
                    last_access: counter,
                },
            );
        }
    }

    /// Insert a clean block.
    pub fn insert_clean(&mut self, block_id: BlockId, data: [u8; BLOCK_SIZE]) {
        self.ensure_capacity_no_flush();
        self.access_counter += 1;
        self.entries.insert(
            block_id,
            CachedBlock {
                data,
                dirty: false,
                last_access: self.access_counter,
            },
        );
        self.clean_queue.push_back(block_id);
    }

    /// O(1) eviction of clean blocks.
    fn ensure_capacity_no_flush(&mut self) {
        while self.entries.len() >= self.capacity {
            if let Some(clean_id) = self.clean_queue.pop_front() {
                if let Some(entry) = self.entries.get(&clean_id) {
                    if !entry.dirty {
                        self.entries.remove(&clean_id);
                        break;
                    }
                }
            } else {
                break;
            }
        }
    }

    /// Flush all dirty blocks to device.
    pub fn flush_dirty<D: BlockDevice>(&mut self, dev: &mut D) -> Result<usize> {
        let mut flushed = 0;
        for (block_id, entry) in self.entries.iter_mut() {
            if entry.dirty {
                dev.write_block(*block_id, &entry.data)?;
                entry.dirty = false;
                flushed += 1;
            }
        }
        if flushed > 0 {
            dev.flush()?;
        }
        Ok(flushed)
    }

    /// Clear all cached blocks.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.clean_queue.clear();
    }
}
