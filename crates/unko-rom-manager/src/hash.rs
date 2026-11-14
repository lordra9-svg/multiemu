//! Content hashes used to match files against DAT entries.

use md5::Md5;
use sha1::{Digest, Sha1};

/// CRC-32, MD5 and SHA-1 for one file or archive member.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Hashes {
    /// Byte length.
    pub size: u64,
    /// CRC-32 (IEEE).
    pub crc: u32,
    /// MD5 digest.
    pub md5: [u8; 16],
    /// SHA-1 digest.
    pub sha1: [u8; 20],
}

impl Hashes {
    /// Hexadecimal MD5.
    #[must_use]
    pub fn md5_hex(&self) -> String {
        hex(&self.md5)
    }

    /// Hexadecimal SHA-1.
    #[must_use]
    pub fn sha1_hex(&self) -> String {
        hex(&self.sha1)
    }

    /// Hexadecimal CRC-32 (`8` digits).
    #[must_use]
    pub fn crc_hex(&self) -> String {
        format!("{:08x}", self.crc)
    }
}

/// Hashes `bytes`.
#[must_use]
pub fn hash_bytes(bytes: &[u8]) -> Hashes {
    let crc = crc32fast::hash(bytes);
    let md5: [u8; 16] = Md5::digest(bytes).into();
    let sha1: [u8; 20] = Sha1::digest(bytes).into();
    Hashes {
        size: bytes.len() as u64,
        crc,
        md5,
        sha1,
    }
}

/// Lowercase hexadecimal.
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Parses a fixed-size hexadecimal digest.
#[must_use]
pub fn parse_digest<const N: usize>(value: &str) -> Option<[u8; N]> {
    let value = value.trim();
    if value.len() != N * 2 {
        return None;
    }
    let mut out = [0u8; N];
    for (index, chunk) in value.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let text = std::str::from_utf8(chunk).ok()?;
        out[index] = u8::from_str_radix(text, 16).ok()?;
    }
    Some(out)
}

/// Parses an 8-digit hexadecimal CRC-32.
#[must_use]
pub fn parse_crc(value: &str) -> Option<u32> {
    u32::from_str_radix(value.trim(), 16).ok()
}

/// Parses a space-separated byte string such as an iNES header.
#[must_use]
pub fn parse_byte_string(value: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    for token in value.split_whitespace() {
        let token = token.trim();
        if token.len() != 2 {
            return None;
        }
        out.push(u8::from_str_radix(token, 16).ok()?);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors() {
        let hashes = hash_bytes(b"123456789");
        assert_eq!(hashes.crc_hex(), "cbf43926");
        assert_eq!(hashes.md5_hex(), "25f9e794323b453885f5181f1b624d0b");
        assert_eq!(
            hashes.sha1_hex(),
            "f7c3bc1d808e04732adf679965ccc34ca7ae3441"
        );
    }

    #[test]
    fn digest_round_trip() {
        let digest = parse_digest::<16>("25f9e794323b453885f5181f1b624d0b").expect("md5");
        assert_eq!(hex(&digest), "25f9e794323b453885f5181f1b624d0b");
        assert!(parse_digest::<20>("too short").is_none());
    }

    #[test]
    fn header_bytes_parse() {
        let bytes = parse_byte_string("4E 45 53 1A").expect("header");
        assert_eq!(bytes, b"NES\x1a");
    }
}
