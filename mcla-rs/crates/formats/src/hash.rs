//! Jenkins one-at-a-time hash, as used by RAGE (`atStringHash`).
//! File names in MC:LA RPF3 archives are hashed after lower-casing.

/// Hash raw bytes (no case folding).
pub fn joaat_bytes(bytes: &[u8]) -> u32 {
    let mut h: u32 = 0;
    for &c in bytes {
        h = h.wrapping_add(c as u32);
        h = h.wrapping_add(h << 10);
        h ^= h >> 6;
    }
    h = h.wrapping_add(h << 3);
    h ^= h >> 11;
    h = h.wrapping_add(h << 15);
    h
}

/// Hash a string the way the game hashes file names: ASCII lower-cased,
/// backslashes normalised to forward slashes.
pub fn joaat(s: &str) -> u32 {
    let mut h: u32 = 0;
    for b in s.bytes() {
        let c = match b {
            b'A'..=b'Z' => b + 32,
            b'\\' => b'/',
            _ => b,
        };
        h = h.wrapping_add(c as u32);
        h = h.wrapping_add(h << 10);
        h ^= h >> 6;
    }
    h = h.wrapping_add(h << 3);
    h ^= h >> 11;
    h = h.wrapping_add(h << 15);
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn known_values() {
        // Recovered by matching executable strings against the archive TOC.
        assert_eq!(joaat("textures"), 0xa26a72dd);
        assert_eq!(joaat("ui"), 0x1b2d32db);
        assert_eq!(joaat("galaxy.dds"), 0xe9b2daa3);
    }
}
