//! High-speed 64-bit hashing for directory lookups and block deduplication.
//! Implements a fast 64-bit hash (xxHash64-style mixer) with zero dependencies.

const PRIME64_1: u64 = 0x9E3779B185EBCA87;
const PRIME64_2: u64 = 0xC2B2AE3D27D4EB4F;
const PRIME64_3: u64 = 0x165667B19E3779F9;
const PRIME64_4: u64 = 0x85EBCA77C2B2AE63;
const PRIME64_5: u64 = 0x27D4EB2F165667C5;

#[inline(always)]
fn rotl(x: u64, r: u32) -> u64 {
    (x << r) | (x >> (64 - r))
}

/// Compute a high-entropy, fast 64-bit hash of a byte slice.
pub fn hash64(data: &[u8]) -> u64 {
    hash64_with_seed(data, 0)
}

/// Compute a fast 64-bit hash with a specified seed.
pub fn hash64_with_seed(data: &[u8], seed: u64) -> u64 {
    let len = data.len() as u64;
    let mut h64: u64;

    if len >= 32 {
        let mut v1 = seed.wrapping_add(PRIME64_1).wrapping_add(PRIME64_2);
        let mut v2 = seed.wrapping_add(PRIME64_2);
        let mut v3 = seed;
        let mut v4 = seed.wrapping_sub(PRIME64_1);

        let mut offset = 0;
        let limit = data.len() - 32;

        while offset <= limit {
            let chunk1 = read_u64_le(&data[offset..offset + 8]);
            v1 = v1.wrapping_add(chunk1.wrapping_mul(PRIME64_2));
            v1 = rotl(v1, 31).wrapping_mul(PRIME64_1);

            let chunk2 = read_u64_le(&data[offset + 8..offset + 16]);
            v2 = v2.wrapping_add(chunk2.wrapping_mul(PRIME64_2));
            v2 = rotl(v2, 31).wrapping_mul(PRIME64_1);

            let chunk3 = read_u64_le(&data[offset + 16..offset + 24]);
            v3 = v3.wrapping_add(chunk3.wrapping_mul(PRIME64_2));
            v3 = rotl(v3, 31).wrapping_mul(PRIME64_1);

            let chunk4 = read_u64_le(&data[offset + 24..offset + 32]);
            v4 = v4.wrapping_add(chunk4.wrapping_mul(PRIME64_2));
            v4 = rotl(v4, 31).wrapping_mul(PRIME64_1);

            offset += 32;
        }

        h64 = rotl(v1, 1).wrapping_add(rotl(v2, 7)).wrapping_add(rotl(v3, 12)).wrapping_add(rotl(v4, 18));

        v1 = rotl(v1.wrapping_mul(PRIME64_2), 31).wrapping_mul(PRIME64_1);
        h64 = (h64 ^ v1).wrapping_mul(PRIME64_1).wrapping_add(PRIME64_4);

        v2 = rotl(v2.wrapping_mul(PRIME64_2), 31).wrapping_mul(PRIME64_1);
        h64 = (h64 ^ v2).wrapping_mul(PRIME64_1).wrapping_add(PRIME64_4);

        v3 = rotl(v3.wrapping_mul(PRIME64_2), 31).wrapping_mul(PRIME64_1);
        h64 = (h64 ^ v3).wrapping_mul(PRIME64_1).wrapping_add(PRIME64_4);

        v4 = rotl(v4.wrapping_mul(PRIME64_2), 31).wrapping_mul(PRIME64_1);
        h64 = (h64 ^ v4).wrapping_mul(PRIME64_1).wrapping_add(PRIME64_4);
    } else {
        h64 = seed.wrapping_add(PRIME64_5);
    }

    h64 = h64.wrapping_add(len);

    let mut remaining = data.len() % 32;
    let mut offset = data.len() - remaining;

    while remaining >= 8 {
        let k1 = read_u64_le(&data[offset..offset + 8]);
        let mut k1 = k1.wrapping_mul(PRIME64_2);
        k1 = rotl(k1, 31).wrapping_mul(PRIME64_1);
        h64 = (h64 ^ k1).wrapping_mul(PRIME64_1).wrapping_add(PRIME64_4);
        offset += 8;
        remaining -= 8;
    }

    if remaining >= 4 {
        let k1 = read_u32_le(&data[offset..offset + 4]) as u64;
        h64 = (h64 ^ (k1.wrapping_mul(PRIME64_1))).wrapping_mul(PRIME64_2).wrapping_add(PRIME64_3);
        offset += 4;
        remaining -= 4;
    }

    while remaining > 0 {
        let k1 = (data[offset] as u64).wrapping_mul(PRIME64_5);
        h64 = (h64 ^ k1).wrapping_mul(PRIME64_1);
        offset += 1;
        remaining -= 1;
    }

    // Final avalanche mix
    h64 ^= h64 >> 33;
    h64 = h64.wrapping_mul(PRIME64_2);
    h64 ^= h64 >> 29;
    h64 = h64.wrapping_mul(PRIME64_3);
    h64 ^= h64 >> 32;

    h64
}

#[inline(always)]
fn read_u64_le(bytes: &[u8]) -> u64 {
    u64::from_le_bytes(bytes[0..8].try_into().unwrap())
}

#[inline(always)]
fn read_u32_le(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes[0..4].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash64() {
        let h1 = hash64(b"hello world");
        let h2 = hash64(b"hello world");
        let h3 = hash64(b"hello world 2");
        assert_eq!(h1, h2);
        assert_ne!(h1, h3);
    }
}
