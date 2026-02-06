//! Binary serialization infrastructure for NanoTDF
//!
//! This module provides traits and utilities for reading and writing
//! binary data in the NanoTDF format. All multi-byte integers use
//! big-endian byte order as specified in the NanoTDF specification.

use std::io::{self, Read, Write};

pub mod traits;

pub use traits::{BinaryRead, BinaryWrite};

/// Read a u8 from a reader
pub fn read_u8<R: Read>(reader: &mut R) -> io::Result<u8> {
    let mut buf = [0u8; 1];
    reader.read_exact(&mut buf)?;
    Ok(buf[0])
}

/// Read a u16 (big-endian) from a reader
pub fn read_u16_be<R: Read>(reader: &mut R) -> io::Result<u16> {
    let mut buf = [0u8; 2];
    reader.read_exact(&mut buf)?;
    Ok(u16::from_be_bytes(buf))
}

/// Read a u24 (big-endian, 3 bytes) from a reader
/// Returns as u32 since Rust doesn't have a u24 type
pub fn read_u24_be<R: Read>(reader: &mut R) -> io::Result<u32> {
    let mut buf = [0u8; 3];
    reader.read_exact(&mut buf)?;
    Ok(u32::from_be_bytes([0, buf[0], buf[1], buf[2]]))
}

/// Read a u32 (big-endian) from a reader
pub fn read_u32_be<R: Read>(reader: &mut R) -> io::Result<u32> {
    let mut buf = [0u8; 4];
    reader.read_exact(&mut buf)?;
    Ok(u32::from_be_bytes(buf))
}

/// Read exactly n bytes from a reader
pub fn read_bytes<R: Read>(reader: &mut R, n: usize) -> io::Result<Vec<u8>> {
    let mut buf = vec![0u8; n];
    reader.read_exact(&mut buf)?;
    Ok(buf)
}

/// Write a u8 to a writer
pub fn write_u8<W: Write>(writer: &mut W, value: u8) -> io::Result<()> {
    writer.write_all(&[value])
}

/// Write a u16 (big-endian) to a writer
pub fn write_u16_be<W: Write>(writer: &mut W, value: u16) -> io::Result<()> {
    writer.write_all(&value.to_be_bytes())
}

/// Write a u24 (big-endian, 3 bytes) to a writer
/// Takes u32 but only writes the lower 24 bits
pub fn write_u24_be<W: Write>(writer: &mut W, value: u32) -> io::Result<()> {
    if value > 0x00FF_FFFF {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Value too large for u24",
        ));
    }
    let bytes = value.to_be_bytes();
    writer.write_all(&bytes[1..4])
}

/// Write a u32 (big-endian) to a writer
pub fn write_u32_be<W: Write>(writer: &mut W, value: u32) -> io::Result<()> {
    writer.write_all(&value.to_be_bytes())
}

/// Write bytes to a writer
pub fn write_bytes<W: Write>(writer: &mut W, bytes: &[u8]) -> io::Result<()> {
    writer.write_all(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_u8_roundtrip() {
        let mut buf = Vec::new();
        write_u8(&mut buf, 0x42).unwrap();
        assert_eq!(buf, vec![0x42]);

        let mut cursor = Cursor::new(buf);
        assert_eq!(read_u8(&mut cursor).unwrap(), 0x42);
    }

    #[test]
    fn test_u16_roundtrip() {
        let mut buf = Vec::new();
        write_u16_be(&mut buf, 0x1234).unwrap();
        assert_eq!(buf, vec![0x12, 0x34]);

        let mut cursor = Cursor::new(buf);
        assert_eq!(read_u16_be(&mut cursor).unwrap(), 0x1234);
    }

    #[test]
    fn test_u24_roundtrip() {
        let mut buf = Vec::new();
        write_u24_be(&mut buf, 0x12_3456).unwrap();
        assert_eq!(buf, vec![0x12, 0x34, 0x56]);

        let mut cursor = Cursor::new(buf);
        assert_eq!(read_u24_be(&mut cursor).unwrap(), 0x12_3456);
    }

    #[test]
    fn test_u24_overflow() {
        let mut buf = Vec::new();
        let result = write_u24_be(&mut buf, 0x01_000000);
        assert!(result.is_err());
    }

    #[test]
    fn test_u32_roundtrip() {
        let mut buf = Vec::new();
        write_u32_be(&mut buf, 0x12345678).unwrap();
        assert_eq!(buf, vec![0x12, 0x34, 0x56, 0x78]);

        let mut cursor = Cursor::new(buf);
        assert_eq!(read_u32_be(&mut cursor).unwrap(), 0x12345678);
    }

    #[test]
    fn test_bytes_roundtrip() {
        let data = vec![0x01, 0x02, 0x03, 0x04, 0x05];
        let mut buf = Vec::new();
        write_bytes(&mut buf, &data).unwrap();
        assert_eq!(buf, data);

        let mut cursor = Cursor::new(buf);
        assert_eq!(read_bytes(&mut cursor, 5).unwrap(), data);
    }
}
