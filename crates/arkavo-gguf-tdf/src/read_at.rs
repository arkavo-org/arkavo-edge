//! Virtual GGUF `read_at` (spec §13.3).
//!
//! Extra anonymous plaintext is the retained header plus one cached weight
//! segment: `headerBytes + maxSegment`. A tag failure is sticky, zeroizes the
//! bytes already copied into the caller's buffer, and never falls back.

use crate::error::GgufTdfError;
use crate::index::SegmentMap;
use base64::Engine as _;
use opentdf::{GgufIndex, TdfEncryption, TdfMemberIndex};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use subtle::ConstantTimeEq;
use zeroize::{Zeroize, Zeroizing};

/// A decrypted view of the virtual GGUF, served one bounded segment at a time.
pub struct VirtualGguf {
    file: File,
    members: TdfMemberIndex,
    index: GgufIndex,
    map: SegmentMap,
    encryption: TdfEncryption,
    /// Segment 0's plaintext, retained so llama.cpp's repeated header reads
    /// never decrypt twice.
    header_plain: Zeroizing<Vec<u8>>,
    /// The single weight-segment scratch. Its length is the cached segment's
    /// plaintext length, never more than `maxSegment`.
    scratch: Zeroizing<Vec<u8>>,
    /// Ciphertext copy-out buffer for the member being decrypted.
    cipher: Vec<u8>,
    /// Base64 GMAC hash per segment, from the manifest, so a decrypt can be
    /// checked against the manifest as well as against the GCM tag.
    hashes: Vec<String>,
    cached: Option<usize>,
    failed: Option<GgufTdfError>,
}

impl VirtualGguf {
    pub(crate) fn new(
        file: File,
        members: TdfMemberIndex,
        index: GgufIndex,
        map: SegmentMap,
        encryption: TdfEncryption,
        header_plain: Zeroizing<Vec<u8>>,
        hashes: Vec<String>,
    ) -> Self {
        Self {
            file,
            members,
            index,
            map,
            encryption,
            header_plain,
            scratch: Zeroizing::new(Vec::new()),
            cipher: Vec::new(),
            hashes,
            cached: None,
            failed: None,
        }
    }

    /// Length of the virtual GGUF.
    pub fn virtual_size(&self) -> u64 {
        self.index.virtual_size
    }

    /// Offset of `tensor_data` in the virtual file.
    pub fn header_bytes(&self) -> u64 {
        self.index.header_bytes
    }

    /// The sticky failure, if a decrypt has failed.
    ///
    /// `read_at` returns 0 for both EOF and failure, so callers that need to
    /// tell them apart consult this.
    pub fn error(&self) -> Option<&GgufTdfError> {
        self.failed.as_ref()
    }

    /// Copies virtual-GGUF bytes at `offset` into `dst`, returning the count.
    ///
    /// Returns 0 on EOF, on an empty destination, and on failure. On failure
    /// the bytes already written to `dst` in this call are zeroized, so a
    /// caller never observes a partial plaintext for a segment that did not
    /// authenticate.
    pub fn read_at(&mut self, offset: u64, dst: &mut [u8]) -> usize {
        if self.failed.is_some() || dst.is_empty() {
            return 0;
        }
        let virtual_size = self.index.virtual_size;
        if offset >= virtual_size {
            return 0;
        }

        let available = (virtual_size - offset) as usize;
        let len = dst.len().min(available);

        let mut written = 0usize;
        while written < len {
            let position = offset + written as u64;
            let Some(id) = self.map.covering(position) else {
                break;
            };
            if let Err(err) = self.ensure_segment(id) {
                dst[..written].zeroize();
                self.scratch.zeroize();
                self.cached = None;
                self.failed = Some(err);
                return 0;
            }

            let start = self.map.start_of(id);
            let local = (position - start) as usize;
            let plaintext = if id == 0 {
                self.header_plain.as_slice()
            } else {
                self.scratch.as_slice()
            };
            if local >= plaintext.len() {
                dst[..written].zeroize();
                self.failed = Some(GgufTdfError::BadIndex(
                    "segment map disagrees with the decrypted segment length".to_string(),
                ));
                return 0;
            }

            let n = (len - written).min(plaintext.len() - local);
            dst[written..written + n].copy_from_slice(&plaintext[local..local + n]);
            written += n;
        }

        written
    }

    /// Makes segment `id`'s plaintext available, decrypting if needed.
    ///
    /// Segment 0 is already retained and authenticated. Any other segment
    /// replaces the single cached weight segment, whose previous plaintext is
    /// zeroized before it is overwritten.
    fn ensure_segment(&mut self, id: usize) -> Result<(), GgufTdfError> {
        if id == 0 || self.cached == Some(id) {
            return Ok(());
        }

        let segment = self
            .index
            .segments
            .get(id)
            .ok_or_else(|| GgufTdfError::BadIndex(format!("no segment {id}")))?;
        let location = self
            .members
            .get(&segment.entry)
            .ok_or_else(|| GgufTdfError::BadIndex(format!("no member {:?}", segment.entry)))?;

        let expected_len = segment.plain + crate::SEGMENT_OVERHEAD;
        if location.size != expected_len {
            return Err(GgufTdfError::TagMismatch);
        }

        self.cipher.clear();
        self.cipher.resize(location.size as usize, 0);
        self.file.seek(SeekFrom::Start(location.data_start))?;
        self.file.read_exact(&mut self.cipher)?;

        // Evict before overwriting so a failed decrypt cannot leave the
        // previous segment's plaintext reachable.
        self.scratch.zeroize();
        self.cached = None;
        self.scratch.resize(segment.plain as usize, 0);

        let tag = self
            .encryption
            .decrypt_segment_into(&self.cipher, &mut self.scratch)
            .map_err(|_| GgufTdfError::TagMismatch)?;

        let row = self
            .index_row(id)
            .ok_or_else(|| GgufTdfError::BadIndex(format!("no integrity row {id}")))?;
        let expected = base64::engine::general_purpose::STANDARD
            .decode(row)
            .map_err(|_| GgufTdfError::TagMismatch)?;
        if expected.ct_eq(&tag).unwrap_u8() != 1 {
            self.scratch.zeroize();
            return Err(GgufTdfError::TagMismatch);
        }

        self.cached = Some(id);
        Ok(())
    }

    fn index_row(&self, id: usize) -> Option<&str> {
        self.hashes.get(id).map(String::as_str)
    }
}

impl Drop for VirtualGguf {
    fn drop(&mut self) {
        // `Zeroizing` clears the plaintext buffers; the ciphertext copy-out is
        // not secret but costs nothing to clear.
        self.cipher.zeroize();
    }
}
