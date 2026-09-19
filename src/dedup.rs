//! Block-level deduplication engine using fast hash fingerprinting.

use std::collections::HashMap;
use crate::hash::hash64;
use crate::types::{BlockId, BLOCK_SIZE};

#[derive(Debug, Clone)]
pub struct DedupEntry {
    pub physical_block: BlockId,
    pub ref_count: u32,
}

pub struct DedupEngine {
    enabled: bool,
    table: HashMap<u64, DedupEntry>,
    saved_blocks: u64,
}

impl DedupEngine {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            table: HashMap::new(),
            saved_blocks: 0,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Check if a block with this content already exists on disk.
    /// Returns Some(existing_physical_block) if deduped.
    pub fn find_or_register(&mut self, block_data: &[u8], new_block_id: BlockId) -> Option<BlockId> {
        if !self.enabled {
            return None;
        }

        // Compute 64-bit content hash
        let hash = hash64(&block_data[..BLOCK_SIZE]);

        if let Some(entry) = self.table.get_mut(&hash) {
            entry.ref_count += 1;
            self.saved_blocks += 1;
            Some(entry.physical_block)
        } else {
            self.table.insert(
                hash,
                DedupEntry {
                    physical_block: new_block_id,
                    ref_count: 1,
                },
            );
            None
        }
    }

    /// Deregister a block reference.
    pub fn release_block(&mut self, hash: u64) -> bool {
        if let Some(entry) = self.table.get_mut(&hash) {
            if entry.ref_count > 1 {
                entry.ref_count -= 1;
                false // still referenced
            } else {
                self.table.remove(&hash);
                true // can be freed
            }
        } else {
            true
        }
    }

    /// Total blocks saved by deduplication.
    pub fn saved_blocks(&self) -> u64 {
        self.saved_blocks
    }

    /// Number of unique fingerprint entries in the dedup table.
    pub fn unique_blocks_count(&self) -> usize {
        self.table.len()
    }
}
