//! CurseForge file fingerprint: MurmurHash2 with seed 1 over the file with whitespace removed.

use std::path::Path;

use crate::error::{IoContext, Result};

const M: u32 = 0x5bd1_e995;
const R: u32 = 24;

/// Bytes CurseForge ignores: tab, LF, CR and space.
fn is_filtered(byte: u8) -> bool {
    matches!(byte, 9 | 10 | 13 | 32)
}

pub fn fingerprint(data: &[u8]) -> u32 {
    let kept: Vec<u8> = data
        .iter()
        .copied()
        .filter(|&byte| !is_filtered(byte))
        .collect();
    // The reference implementation keeps the length as u32, so it wraps for inputs over 4 GiB.
    let mut hash = 1 ^ kept.len() as u32;

    let (chunks, tail) = kept.as_chunks::<4>();
    for &chunk in chunks {
        let mut k = u32::from_le_bytes(chunk);
        k = k.wrapping_mul(M);
        k ^= k >> R;
        k = k.wrapping_mul(M);
        hash = hash.wrapping_mul(M) ^ k;
    }

    if !tail.is_empty() {
        for (i, &byte) in tail.iter().enumerate() {
            hash ^= u32::from(byte) << (8 * i);
        }
        hash = hash.wrapping_mul(M);
    }

    hash ^= hash >> 13;
    hash = hash.wrapping_mul(M);
    hash ^ (hash >> 15)
}

pub fn fingerprint_file(path: &Path) -> Result<u32> {
    Ok(fingerprint(&std::fs::read(path).at(path)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Expected values come from an independent C implementation of MurmurHash2 (seed 1) run over the
    /// input with bytes 9, 10, 13 and 32 removed, covering every remainder of length % 4.
    #[test]
    fn matches_reference_vectors() {
        let vectors: [(&[u8], u32); 12] = [
            (b"", 1540447798),
            (b"a", 626045324),
            (b"ab", 1692487918),
            (b"abc", 1621425345),
            (b"abcd", 3376380438),
            (b"abcde", 3469237630),
            (b"Hello, World!", 1961219979),
            (b"  \t\r\n a b\nc d  ", 3376380438),
            (
                b"{\n  \"id\": \"sodium\",\n  \"version\": \"0.5.13\"\n}\n",
                3761974559,
            ),
            (b"The quick brown fox jumps over the lazy dog", 3751777527),
            (b"   ", 1540447798),
            (b"\xff\xfe\x80zz1234567", 4150505437),
        ];
        for (input, expected) in vectors {
            assert_eq!(fingerprint(input), expected, "input {input:?}");
        }
    }
}
