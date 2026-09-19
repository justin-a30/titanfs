//! GhostTrees™: Merkle-Deduplicated Virtual Workspace Branching.
//! Instant O(1) cloning and atomic deletion of multi-gigabyte directory trees.

use std::collections::HashMap;
use crate::directory::DirEntry;
use crate::hash::hash64;
use crate::types::{FsError, InodeId, Result};

#[derive(Debug, Clone)]
pub struct GhostBranch {
    pub name: String,
    pub source_inode: InodeId,
    pub cloned_inode: InodeId,
    pub merkle_root: u64,
    pub ref_count: u32,
}

pub struct GhostTreeManager {
    branches: HashMap<String, GhostBranch>,
    merkle_roots: HashMap<InodeId, u64>,
}

impl GhostTreeManager {
    pub fn new() -> Self {
        Self {
            branches: HashMap::new(),
            merkle_roots: HashMap::new(),
        }
    }

    /// Calculate the Merkle hash of a directory based on its entries.
    pub fn compute_merkle_root(&mut self, dir_inode_id: InodeId, entries: &[DirEntry]) -> u64 {
        let mut buf = Vec::with_capacity(entries.len() * 32);
        for entry in entries {
            buf.extend_from_slice(&entry.inode_id.to_le_bytes());
            buf.push(entry.file_type as u8);
            buf.extend_from_slice(&entry.hash.to_le_bytes());
        }
        let root_hash = hash64(&buf);
        self.merkle_roots.insert(dir_inode_id, root_hash);
        root_hash
    }

    /// Register a branched directory.
    pub fn register_branch(
        &mut self,
        branch_name: &str,
        source_inode: InodeId,
        cloned_inode: InodeId,
        merkle_root: u64,
    ) -> Result<()> {
        if self.branches.contains_key(branch_name) {
            return Err(FsError::AlreadyExists(format!("GhostTree branch '{}'", branch_name)));
        }

        self.branches.insert(
            branch_name.to_string(),
            GhostBranch {
                name: branch_name.to_string(),
                source_inode,
                cloned_inode,
                merkle_root,
                ref_count: 1,
            },
        );

        Ok(())
    }

    /// Get metadata for a branch.
    pub fn get_branch(&self, branch_name: &str) -> Option<&GhostBranch> {
        self.branches.get(branch_name)
    }

    /// List all active GhostTree branches.
    pub fn list_branches(&self) -> Vec<&GhostBranch> {
        self.branches.values().collect()
    }

    /// Remove a branch mapping.
    pub fn unregister_branch(&mut self, branch_name: &str) -> Option<GhostBranch> {
        self.branches.remove(branch_name)
    }
}
