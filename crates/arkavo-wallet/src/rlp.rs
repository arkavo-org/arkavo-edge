//! RLP (Recursive Length Prefix) encoding utilities for EVM transactions

use alloy_primitives::U256;

/// Encode a u64 value in RLP format
pub fn encode_u64(val: u64, out: &mut Vec<u8>) {
    if val == 0 {
        out.push(0x80);
    } else if val < 128 {
        out.push(val as u8);
    } else {
        let bytes = val.to_be_bytes();
        let start = bytes.iter().position(|&b| b != 0).unwrap_or(8);
        let len = 8 - start;
        out.push(0x80 + len as u8);
        out.extend_from_slice(&bytes[start..]);
    }
}

/// Encode a U256 value in RLP format
pub fn encode_u256(val: &U256, out: &mut Vec<u8>) {
    if val.is_zero() {
        out.push(0x80);
    } else {
        let bytes: [u8; 32] = val.to_be_bytes();
        let start = bytes.iter().position(|&b| b != 0).unwrap_or(32);
        let trimmed = &bytes[start..];
        if trimmed.len() == 1 && trimmed[0] < 128 {
            out.push(trimmed[0]);
        } else {
            out.push(0x80 + trimmed.len() as u8);
            out.extend_from_slice(trimmed);
        }
    }
}

/// Encode a byte slice in RLP format
pub fn encode_bytes(bytes: &[u8], out: &mut Vec<u8>) {
    if bytes.is_empty() {
        out.push(0x80);
    } else if bytes.len() == 1 && bytes[0] < 128 {
        out.push(bytes[0]);
    } else if bytes.len() < 56 {
        out.push(0x80 + bytes.len() as u8);
        out.extend_from_slice(bytes);
    } else {
        let len_bytes = encode_length(bytes.len());
        out.push(0xb7 + len_bytes.len() as u8);
        out.extend_from_slice(&len_bytes);
        out.extend_from_slice(bytes);
    }
}

/// Encode an RLP list
pub fn encode_list(items: &[u8], out: &mut Vec<u8>) {
    if items.len() < 56 {
        out.push(0xc0 + items.len() as u8);
    } else {
        let len_bytes = encode_length(items.len());
        out.push(0xf7 + len_bytes.len() as u8);
        out.extend_from_slice(&len_bytes);
    }
    out.extend_from_slice(items);
}

fn encode_length(len: usize) -> Vec<u8> {
    if len == 0 {
        return vec![];
    }
    let mut bytes = Vec::new();
    let mut n = len;
    while n > 0 {
        bytes.push((n & 0xff) as u8);
        n >>= 8;
    }
    bytes.reverse();
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_u64() {
        let mut out = Vec::new();
        encode_u64(0, &mut out);
        assert_eq!(out, vec![0x80]);

        out.clear();
        encode_u64(127, &mut out);
        assert_eq!(out, vec![127]);

        out.clear();
        encode_u64(128, &mut out);
        assert_eq!(out, vec![0x81, 128]);

        out.clear();
        encode_u64(256, &mut out);
        assert_eq!(out, vec![0x82, 1, 0]);
    }

    #[test]
    fn test_encode_bytes() {
        let mut out = Vec::new();
        encode_bytes(&[], &mut out);
        assert_eq!(out, vec![0x80]);

        out.clear();
        encode_bytes(&[0x42], &mut out);
        assert_eq!(out, vec![0x42]);

        out.clear();
        encode_bytes(&[0x80], &mut out);
        assert_eq!(out, vec![0x81, 0x80]);
    }

    #[test]
    fn test_encode_u256_zero() {
        let mut out = Vec::new();
        encode_u256(&U256::ZERO, &mut out);
        assert_eq!(out, vec![0x80]);
    }

    #[test]
    fn test_encode_u256_small() {
        let mut out = Vec::new();
        encode_u256(&U256::from(127u64), &mut out);
        assert_eq!(out, vec![127]);
    }
}
