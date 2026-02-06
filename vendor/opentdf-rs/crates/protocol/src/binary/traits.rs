//! Traits for binary serialization and deserialization

use std::io::{self, Read, Write};

/// Trait for types that can be read from binary format
pub trait BinaryRead: Sized {
    /// Read this type from a binary reader
    fn read_from<R: Read>(reader: &mut R) -> io::Result<Self>;
}

/// Trait for types that can be written to binary format
pub trait BinaryWrite {
    /// Write this type to a binary writer
    fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<()>;

    /// Get the size in bytes when serialized
    fn serialized_size(&self) -> usize;
}

/// Trait for types that support both reading and writing
pub trait BinarySerialize: BinaryRead + BinaryWrite {}

// Blanket implementation
impl<T: BinaryRead + BinaryWrite> BinarySerialize for T {}
