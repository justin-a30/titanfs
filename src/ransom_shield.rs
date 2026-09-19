//! RansomShield™: Hardware-Entropy Quorum & Autonomous Quarantine Engine.
//! Monitors Shannon entropy of block writes to detect, intercept, and quarantine ransomware.

use std::collections::HashMap;
use std::time::{Duration, Instant};
use crate::types::{BlockId, InodeId, BLOCK_SIZE};

/// High-entropy threshold indicating encrypted ciphertext (typical text/binaries are 3.5 - 6.2).
pub const RANSOM_ENTROPY_THRESHOLD: f64 = 7.50;

/// Number of high-entropy rapid writes required to trigger automated lockdown.
pub const RANSOM_QUORUM_BURST: usize = 3;

/// Rolling time window for anomaly correlation.
pub const RANSOM_WINDOW_SECS: u64 = 5;

/// Compute Shannon entropy (bits per byte, 0.0 to 8.0) of a byte slice.
pub fn calculate_shannon_entropy(data: &[u8]) -> f64 {
    if data.is_empty() {
        return 0.0;
    }

    let mut freq = [0usize; 256];
    for &b in data {
        freq[b as usize] += 1;
    }

    let total = data.len() as f64;
    let mut entropy = 0.0;

    for &count in &freq {
        if count > 0 {
            let p = (count as f64) / total;
            entropy -= p * p.log2();
        }
    }

    entropy
}

#[derive(Debug, Clone)]
pub struct SuspiciousWrite {
    pub inode_id: InodeId,
    pub path: String,
    pub entropy: f64,
    pub timestamp: Instant,
}

#[derive(Debug, Clone)]
pub struct QuarantineReport {
    pub locked: bool,
    pub detected_threats: usize,
    pub quarantined_blocks: usize,
    pub last_entropy: f64,
    pub suspicious_paths: Vec<String>,
}

pub struct RansomShield {
    enabled: bool,
    quarantine_locked: bool,
    recent_suspicious: Vec<SuspiciousWrite>,
    quarantined_blocks: HashMap<BlockId, [u8; BLOCK_SIZE]>,
    quarantined_paths: Vec<String>,
}

impl RansomShield {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            quarantine_locked: false,
            recent_suspicious: Vec::new(),
            quarantined_blocks: HashMap::new(),
            quarantined_paths: Vec::new(),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn is_quarantine_active(&self) -> bool {
        self.quarantine_locked
    }

    /// Inspect written block for cryptographic anomalies.
    /// Returns true if write is quarantined/intercepted, false if benign.
    pub fn inspect_and_filter(
        &mut self,
        inode_id: InodeId,
        path: &str,
        block_id: BlockId,
        data: &[u8],
    ) -> bool {
        if !self.enabled {
            return false;
        }

        let entropy = calculate_shannon_entropy(data);

        // If quarantine is already locked, intercept all writes to preserve originals
        if self.quarantine_locked {
            let mut buf = [0u8; BLOCK_SIZE];
            let len = data.len().min(BLOCK_SIZE);
            buf[..len].copy_from_slice(&data[..len]);
            self.quarantined_blocks.insert(block_id, buf);
            if !self.quarantined_paths.contains(&path.to_string()) {
                self.quarantined_paths.push(path.to_string());
            }
            return true;
        }

        if entropy >= RANSOM_ENTROPY_THRESHOLD {
            let now = Instant::now();
            // Prune expired entries outside rolling window
            self.recent_suspicious.retain(|s| now.duration_since(s.timestamp) < Duration::from_secs(RANSOM_WINDOW_SECS));

            self.recent_suspicious.push(SuspiciousWrite {
                inode_id,
                path: path.to_string(),
                entropy,
                timestamp: now,
            });

            // Check if quorum of distinct files reached
            let distinct_inodes: std::collections::HashSet<InodeId> =
                self.recent_suspicious.iter().map(|s| s.inode_id).collect();

            if distinct_inodes.len() >= RANSOM_QUORUM_BURST {
                // Trigger automated lockdown!
                self.quarantine_locked = true;
                let mut buf = [0u8; BLOCK_SIZE];
                let len = data.len().min(BLOCK_SIZE);
                buf[..len].copy_from_slice(&data[..len]);
                self.quarantined_blocks.insert(block_id, buf);
                self.quarantined_paths.push(path.to_string());
                return true;
            }
        }

        false
    }

    /// Discard quarantined malicious writes and unlock normal operation.
    pub fn discard_quarantine(&mut self) -> usize {
        let count = self.quarantined_blocks.len();
        self.quarantined_blocks.clear();
        self.quarantined_paths.clear();
        self.recent_suspicious.clear();
        self.quarantine_locked = false;
        count
    }

    /// Status report for administration and CLI.
    pub fn get_report(&self) -> QuarantineReport {
        let last_entropy = self.recent_suspicious.last().map(|s| s.entropy).unwrap_or(0.0);
        QuarantineReport {
            locked: self.quarantine_locked,
            detected_threats: self.recent_suspicious.len(),
            quarantined_blocks: self.quarantined_blocks.len(),
            last_entropy,
            suspicious_paths: self.quarantined_paths.clone(),
        }
    }
}
