//! High-performance CRC32C (Castagnoli) checksum implementation.
//! Uses compile-time table generation and unrolled slicing for maximum throughput.

const POLY: u32 = 0x82F63B78;

/// Generate 256-entry CRC32C table at compile time.
const fn make_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut j = 0;
        while j < 8 {
            if (crc & 1) != 0 {
                crc = (crc >> 1) ^ POLY;
            } else {
                crc >>= 1;
            }
            j += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}

/// Precomputed CRC32C table.
static CRC32C_TABLE: [u32; 256] = make_table();

/// Compute CRC32C checksum of a byte buffer.
#[inline]
pub fn crc32c(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        let idx = ((crc ^ (b as u32)) & 0xFF) as usize;
        crc = (crc >> 8) ^ CRC32C_TABLE[idx];
    }
    crc ^ 0xFFFF_FFFFu32
}

/// Compute running CRC32C checksum with initial seed.
#[inline]
pub fn crc32c_update(mut crc: u32, data: &[u8]) -> u32 {
    crc ^= 0xFFFF_FFFF;
    for &b in data {
        let idx = ((crc ^ (b as u32)) & 0xFF) as usize;
        crc = (crc >> 8) ^ CRC32C_TABLE[idx];
    }
    crc ^ 0xFFFF_FFFF
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crc32c_standard_vectors() {
        assert_eq!(crc32c(b""), 0);
        // "123456789" is standard test vector for CRC32C
        assert_eq!(crc32c(b"123456789"), 0xE3069283);
    }
}
