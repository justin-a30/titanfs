//! Instant Copy-on-Write snapshot management.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use crate::inode::Inode;
use crate::types::{FsError, Result};

#[derive(Debug, Clone)]
pub struct SnapshotMeta {
    pub name: String,
    pub generation: u64,
    pub created_at_ns: u64,
    pub root_inode: Inode,
    pub inodes: Vec<Inode>,
}

pub struct SnapshotManager {
    snapshots: HashMap<String, SnapshotMeta>,
}

impl SnapshotManager {
    pub fn new() -> Self {
        Self {
            snapshots: HashMap::new(),
        }
    }

    /// Create a new snapshot capturing current filesystem state.
    pub fn create(&mut self, name: &str, root_inode: &Inode, inodes: Vec<Inode>, generation: u64) -> Result<()> {
        if self.snapshots.contains_key(name) {
            return Err(FsError::AlreadyExists(format!("Snapshot '{}'", name)));
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;

        let meta = SnapshotMeta {
            name: name.to_string(),
            generation,
            created_at_ns: now,
            root_inode: root_inode.clone(),
            inodes,
        };

        self.snapshots.insert(name.to_string(), meta);
        Ok(())
    }

    /// Get snapshot metadata by name.
    pub fn get(&self, name: &str) -> Option<&SnapshotMeta> {
        self.snapshots.get(name)
    }

    /// List all snapshots.
    pub fn list(&self) -> Vec<&SnapshotMeta> {
        self.snapshots.values().collect()
    }

    /// Delete snapshot by name.
    pub fn delete(&mut self, name: &str) -> Result<()> {
        if self.snapshots.remove(name).is_some() {
            Ok(())
        } else {
            Err(FsError::SnapshotNotFound(name.to_string()))
        }
    }
}
