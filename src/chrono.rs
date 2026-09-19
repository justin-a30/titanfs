//! ChronoVFS™: Continuous Microsecond Time-Travel Engine.
//! Records fine-grained temporal version deltas, enabling query syntax like `/path@-15m`.

use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use crate::extent::Extent;
use crate::inode::{Inode, DIRECT_EXTENTS, INLINE_DATA_CAPACITY};
use crate::types::{FsError, InodeId, Result};

///// Snapshot of an inode's storage state at an exact point in time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemporalVersion {
    pub timestamp_ns: u64,
    pub size: u64,
    pub is_inline: bool,
    pub inline_size: u16,
    pub inline_data: [u8; INLINE_DATA_CAPACITY],
    pub extents: [Extent; DIRECT_EXTENTS],
    pub indirect_extent_block: u64,
    pub data_payload: Option<Vec<u8>>,
}

impl TemporalVersion {
    pub fn from_inode(inode: &Inode, timestamp_ns: u64, data_payload: Option<Vec<u8>>) -> Self {
        Self {
            timestamp_ns,
            size: inode.size,
            is_inline: inode.is_inline(),
            inline_size: inode.inline_size,
            inline_data: inode.inline_data,
            extents: inode.extents,
            indirect_extent_block: inode.indirect_extent_block,
            data_payload,
        }
    }

    pub fn apply_to_inode(&self, inode: &mut Inode) {
        inode.size = self.size;
        inode.set_inline(self.is_inline);
        inode.inline_size = self.inline_size;
        inode.inline_data = self.inline_data;
        inode.extents = self.extents;
        inode.indirect_extent_block = self.indirect_extent_block;
    }
}

/// ChronoVFS engine maintaining microsecond historical timelines for inodes.
pub struct ChronoEngine {
    enabled: bool,
    history: HashMap<InodeId, Vec<TemporalVersion>>,
    max_versions_per_inode: usize,
}

impl ChronoEngine {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            history: HashMap::new(),
            max_versions_per_inode: 256,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Record a new temporal version before or after an inode modification.
    pub fn record_version(&mut self, inode: &Inode, payload: Option<Vec<u8>>) {
        if !self.enabled {
            return;
        }

        let now_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;

        let version = TemporalVersion::from_inode(inode, now_ns, payload);
        let list = self.history.entry(inode.id).or_insert_with(Vec::new);

        // Deduplicate if identical state was just recorded
        if let Some(last) = list.last() {
            if last.size == version.size
                && last.inline_data == version.inline_data
                && last.extents == version.extents
                && last.data_payload == version.data_payload
            {
                return;
            }
        }

        if list.len() >= self.max_versions_per_inode {
            list.remove(0); // Evict oldest version
        }
        list.push(version);
    }

    /// Find the historical state active at a specific timestamp.
    pub fn get_version_at(&self, inode_id: InodeId, target_ns: u64) -> Option<&TemporalVersion> {
        let list = self.history.get(&inode_id)?;
        // Find newest version that was recorded at or before target_ns
        list.iter()
            .rev()
            .find(|v| v.timestamp_ns <= target_ns)
            .or_else(|| list.first())
    }

    /// List all available historical versions for an inode.
    pub fn list_versions(&self, inode_id: InodeId) -> Vec<&TemporalVersion> {
        self.history.get(&inode_id)
            .map(|l| l.iter().collect())
            .unwrap_or_default()
    }
}

/// Parse a temporal path like `/docs/file.txt@-15m`, `/docs/file.txt@-500ms`, or `/docs/file.txt@1758283640000000000`.
/// Returns (clean_path, Option<target_timestamp_ns>).
pub fn parse_temporal_path(path: &str) -> Result<(String, Option<u64>)> {
    if let Some(pos) = path.rfind('@') {
        let base_path = &path[..pos];
        let modifier = &path[pos + 1..];

        let now_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;

        if modifier.starts_with('-') {
            // Relative duration syntax: e.g. -500ms, -100us, -5s, -15m, -2h, -1d
            let dur_str = &modifier[1..];
            let target_ns = parse_duration_offset(dur_str, now_ns)?;
            return Ok((base_path.to_string(), Some(target_ns)));
        } else if let Ok(ns) = modifier.parse::<u64>() {
            // Absolute epoch nanoseconds
            return Ok((base_path.to_string(), Some(ns)));
        }
    }

    Ok((path.to_string(), None))
}

fn parse_duration_offset(dur_str: &str, now_ns: u64) -> Result<u64> {
    if dur_str.is_empty() {
        return Err(FsError::InvalidPath("Empty temporal duration modifier".into()));
    }

    let (val_str, unit) = if dur_str.ends_with("ms") {
        (&dur_str[..dur_str.len() - 2], "ms")
    } else if dur_str.ends_with("us") {
        (&dur_str[..dur_str.len() - 2], "us")
    } else if dur_str.ends_with("ns") {
        (&dur_str[..dur_str.len() - 2], "ns")
    } else {
        dur_str.split_at(dur_str.len() - 1)
    };

    let val: u64 = val_str.parse()
        .map_err(|_| FsError::InvalidPath(format!("Invalid duration number: {}", val_str)))?;

    let offset_dur = match unit {
        "ns" => Duration::from_nanos(val),
        "us" => Duration::from_micros(val),
        "ms" => Duration::from_millis(val),
        "s" => Duration::from_secs(val),
        "m" => Duration::from_secs(val * 60),
        "h" => Duration::from_secs(val * 3600),
        "d" => Duration::from_secs(val * 86400),
        _ => return Err(FsError::InvalidPath(format!("Unknown time unit '{}' in duration. Use ns, us, ms, s, m, h, or d.", unit))),
    };

    let offset_ns = offset_dur.as_nanos() as u64;
    Ok(now_ns.saturating_sub(offset_ns))
}
