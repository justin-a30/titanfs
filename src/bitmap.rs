//! Word-aligned bitmap allocator for blocks and inodes.
//! Leverages hardware bit-manipulation instructions (trailing_zeros) for ultra-fast allocations.

use crate::types::BLOCK_SIZE;

/// Bitmap tracking allocation status of blocks or inodes.
/// 0 = free, 1 = allocated.
pub struct Bitmap {
    data: Vec<u8>,
    total_bits: u64,
    dirty: bool,
}

impl Bitmap {
    /// Create a new bitmap initialized with all bits 0 (free).
    pub fn new(total_bits: u64) -> Self {
        let total_bytes = ((total_bits + 7) / 8) as usize;
        // Round up to multiple of BLOCK_SIZE for disk alignment
        let aligned_bytes = ((total_bytes + BLOCK_SIZE - 1) / BLOCK_SIZE) * BLOCK_SIZE;
        Self {
            data: vec![0u8; aligned_bytes],
            total_bits,
            dirty: false,
        }
    }

    /// Construct bitmap from raw disk bytes.
    pub fn from_bytes(bytes: Vec<u8>, total_bits: u64) -> Self {
        Self {
            data: bytes,
            total_bits,
            dirty: false,
        }
    }

    /// Underlying byte slice.
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    /// Mutable access to underlying byte slice.
    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        self.dirty = true;
        &mut self.data
    }

    /// Check if bitmap has pending dirty writes.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Clear the dirty flag.
    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

    /// Test if bit at index is set (allocated).
    #[inline]
    pub fn test(&self, index: u64) -> bool {
        if index >= self.total_bits {
            return true; // treat out-of-bounds as allocated
        }
        let byte_idx = (index / 8) as usize;
        let bit_idx = (index % 8) as u8;
        (self.data[byte_idx] & (1 << bit_idx)) != 0
    }

    /// Set bit at index (mark as allocated).
    #[inline]
    pub fn set(&mut self, index: u64) {
        if index < self.total_bits {
            let byte_idx = (index / 8) as usize;
            let bit_idx = (index % 8) as u8;
            self.data[byte_idx] |= 1 << bit_idx;
            self.dirty = true;
        }
    }

    /// Clear bit at index (mark as free).
    #[inline]
    pub fn clear(&mut self, index: u64) {
        if index < self.total_bits {
            let byte_idx = (index / 8) as usize;
            let bit_idx = (index % 8) as u8;
            self.data[byte_idx] &= !(1 << bit_idx);
            self.dirty = true;
        }
    }

    /// Find the first free bit, set it, and return its index.
    pub fn allocate_first(&mut self) -> Option<u64> {
        let num_u64 = self.data.len() / 8;
        for i in 0..num_u64 {
            let offset = i * 8;
            let word = u64::from_le_bytes(self.data[offset..offset + 8].try_into().unwrap());
            if word != 0xFFFF_FFFF_FFFF_FFFF {
                let free_bit = (!word).trailing_zeros() as u64;
                let index = (i as u64) * 64 + free_bit;
                if index < self.total_bits {
                    self.set(index);
                    return Some(index);
                }
            }
        }

        // Check tail bytes
        let checked_bits = (num_u64 as u64) * 64;
        for index in checked_bits..self.total_bits {
            if !self.test(index) {
                self.set(index);
                return Some(index);
            }
        }

        None
    }

    /// Find a contiguous range of `count` free bits (for extent allocation).
    /// Returns the starting bit index if found, and marks them allocated.
    pub fn allocate_contiguous(&mut self, count: usize) -> Option<u64> {
        if count == 0 {
            return None;
        }
        if count == 1 {
            return self.allocate_first();
        }

        let mut run_start = 0u64;
        let mut run_len = 0usize;
        let mut idx = 0u64;

        while idx < self.total_bits {
            // Fast scan 64 bits if aligned and all 1s
            if run_len == 0 && (idx % 64 == 0) && (idx + 64 <= self.total_bits) {
                let byte_idx = (idx / 8) as usize;
                let word = u64::from_le_bytes(self.data[byte_idx..byte_idx + 8].try_into().unwrap());
                if word == 0xFFFF_FFFF_FFFF_FFFF {
                    idx += 64;
                    continue;
                }
            }

            if !self.test(idx) {
                if run_len == 0 {
                    run_start = idx;
                }
                run_len += 1;
                if run_len == count {
                    // Mark all in run as allocated
                    for b in run_start..run_start + (count as u64) {
                        self.set(b);
                    }
                    return Some(run_start);
                }
            } else {
                run_len = 0;
            }
            idx += 1;
        }

        None
    }

    /// Count total allocated bits.
    pub fn count_allocated(&self) -> u64 {
        let mut count = 0u64;
        let num_u64 = self.data.len() / 8;
        for i in 0..num_u64 {
            let offset = i * 8;
            let word = u64::from_le_bytes(self.data[offset..offset + 8].try_into().unwrap());
            count += word.count_ones() as u64;
        }
        let checked_bytes = num_u64 * 8;
        for &b in &self.data[checked_bytes..] {
            count += b.count_ones() as u64;
        }
        count.min(self.total_bits)
    }

    /// Count total free bits.
    pub fn count_free(&self) -> u64 {
        self.total_bits.saturating_sub(self.count_allocated())
    }
}
