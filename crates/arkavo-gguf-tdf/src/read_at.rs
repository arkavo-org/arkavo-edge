//! Virtual GGUF `read_at` (spec §13.3).
//!
//! Extra anonymous plaintext is the retained header plus up to
//! `cached_segments` decrypted weight segments: `headerBytes + k·maxSegment`.
//! A tag failure is sticky, zeroizes the bytes already copied into the
//! caller's buffer, clears the whole segment cache, and never falls back.

use crate::error::GgufTdfError;
use crate::index::SegmentMap;
use crate::segment_cache::SegmentCache;
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
    /// LRU of decrypted weight segments (spec §13.3).
    cache: SegmentCache,
    /// Ciphertext copy-out buffer for the member being decrypted.
    cipher: Vec<u8>,
    /// Base64 GMAC hash per segment, from the manifest, so a decrypt can be
    /// checked against the manifest as well as against the GCM tag.
    hashes: Vec<String>,
    /// Weight segments decrypted so far, for `segments_decrypted`.
    decrypts: u64,
    failed: Option<GgufTdfError>,
}

impl VirtualGguf {
    // One argument per field the caller has already authenticated or
    // computed at unlock; splitting them into a builder would just move the
    // same eight values one level up without adding a real invariant.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        file: File,
        members: TdfMemberIndex,
        index: GgufIndex,
        map: SegmentMap,
        encryption: TdfEncryption,
        header_plain: Zeroizing<Vec<u8>>,
        hashes: Vec<String>,
        cached_segments: usize,
    ) -> Self {
        Self {
            file,
            members,
            index,
            map,
            encryption,
            header_plain,
            cache: SegmentCache::new(cached_segments),
            cipher: Vec::new(),
            hashes,
            decrypts: 0,
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

    /// Weight segments decrypted so far (§18 observability; tests assert on it).
    pub fn segments_decrypted(&self) -> u64 {
        self.decrypts
    }

    /// Weight segments currently held in plaintext.
    pub fn cached_segments(&self) -> usize {
        self.cache.len()
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
                self.cache.clear();
                self.failed = Some(err);
                return 0;
            }

            let start = self.map.start_of(id);
            let local = (position - start) as usize;
            // Computed without borrowing `self.cache`, so the failure branch
            // below is free to clear it: for id 0 this is the retained
            // header's own length; otherwise it's the length `ensure_segment`
            // sized the cache slot to when this segment was decrypted, which
            // is exactly what's cached under `id` right now.
            let plain_len = if id == 0 {
                self.header_plain.len()
            } else {
                self.index
                    .segments
                    .get(id)
                    .map(|s| s.plain as usize)
                    .unwrap_or(0)
            };
            if local >= plain_len {
                dst[..written].zeroize();
                self.cache.clear();
                self.failed = Some(GgufTdfError::BadIndex(
                    "segment map disagrees with the decrypted segment length".to_string(),
                ));
                return 0;
            }

            let plaintext = if id == 0 {
                self.header_plain.as_slice()
            } else {
                match self.cache.get(id) {
                    Some(plain) => plain,
                    None => {
                        // ensure_segment just inserted or promoted `id`, so
                        // this should be unreachable; read_at is driven from
                        // llama.cpp's FFI read callback, where unwinding
                        // across the boundary is undefined behaviour, so fail
                        // closed instead of panicking.
                        dst[..written].zeroize();
                        self.cache.clear();
                        self.failed = Some(GgufTdfError::BadIndex(format!(
                            "segment {id} vanished from the cache immediately after being cached"
                        )));
                        return 0;
                    }
                }
            };

            let n = (len - written).min(plain_len - local);
            dst[written..written + n].copy_from_slice(&plaintext[local..local + n]);
            written += n;
        }

        written
    }

    /// Makes segment `id`'s plaintext available, decrypting if needed.
    ///
    /// Segment 0 is already retained and authenticated. Any other segment is
    /// served from the LRU cache when present, or decrypted and inserted,
    /// evicting the least-recently-used entry when the cache is full.
    fn ensure_segment(&mut self, id: usize) -> Result<(), GgufTdfError> {
        if id == 0 || self.cache.get(id).is_some() {
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

        // The slot comes back zeroized; a failed decrypt below drops it
        // without ever inserting, so no partial plaintext stays reachable.
        let mut plain = self.cache.take_slot(segment.plain as usize);
        let tag = self
            .encryption
            .decrypt_segment_into(&self.cipher, &mut plain)
            .map_err(|_| GgufTdfError::TagMismatch)?;
        self.decrypts += 1;

        let row = self
            .index_row(id)
            .ok_or_else(|| GgufTdfError::BadIndex(format!("no integrity row {id}")))?;
        let expected = base64::engine::general_purpose::STANDARD
            .decode(row)
            .map_err(|_| GgufTdfError::TagMismatch)?;
        if expected.ct_eq(&tag).unwrap_u8() != 1 {
            return Err(GgufTdfError::TagMismatch);
        }

        self.cache.insert(id, plain);
        Ok(())
    }

    fn index_row(&self, id: usize) -> Option<&str> {
        self.hashes.get(id).map(String::as_str)
    }
}

impl Drop for VirtualGguf {
    fn drop(&mut self) {
        // `Zeroizing` clears the header and cached plaintext buffers; the
        // ciphertext copy-out is not secret but costs nothing to clear.
        self.cipher.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentdf::{GgufSegment, GgufSegmentKind};
    use std::io::Cursor;

    /// A structurally valid, member-less zip. Sufficient here because this
    /// module's tests only ever touch segment 0 (the retained header), which
    /// `read_at` serves from `header_plain` without consulting `members`.
    fn empty_member_index() -> TdfMemberIndex {
        let mut buf = Vec::new();
        zip::ZipWriter::new(Cursor::new(&mut buf)).finish().unwrap();
        TdfMemberIndex::open(Cursor::new(buf)).unwrap()
    }

    /// The index's segment map says the header covers 64 virtual bytes, but
    /// the plaintext actually retained from decryption (`header_plain`) is
    /// only 8 — an internal inconsistency `read_at`'s bounds check exists to
    /// catch. It must fail closed, zeroize what it already copied, and clear
    /// the (here empty) cache rather than panic or read past `header_plain`.
    #[test]
    fn segment_map_disagreeing_with_header_plain_length_fails_closed_and_clears_cache() {
        let index = GgufIndex {
            profile: crate::PROFILE.to_string(),
            alignment: 32,
            header_bytes: 64,
            virtual_size: 64,
            max_segment: crate::DEFAULT_MAX_SEGMENT,
            tensors: Vec::new(),
            segments: vec![GgufSegment {
                id: 0,
                kind: GgufSegmentKind::Header,
                plain: 64,
                entry: crate::HEADER_ENTRY.to_string(),
            }],
        };
        let map = SegmentMap::new(&index);

        let mut vg = VirtualGguf::new(
            File::open("/dev/null").unwrap(),
            empty_member_index(),
            index,
            map,
            TdfEncryption::new().unwrap(),
            Zeroizing::new(vec![0u8; 8]), // shorter than the map's 64-byte claim
            Vec::new(),
            4,
        );

        let mut buf = [0xAAu8; 16];
        assert_eq!(vg.read_at(0, &mut buf), 0);
        assert!(matches!(vg.error(), Some(GgufTdfError::BadIndex(_))));
        assert_eq!(vg.cached_segments(), 0, "cache must be cleared on failure");
        // The first 8 bytes were genuinely copied from header_plain before
        // the second chunk's bounds check failed; they must be wiped too.
        assert!(buf[..8].iter().all(|b| *b == 0));
        assert_eq!(vg.read_at(0, &mut buf), 0, "failure is sticky");
    }
}
