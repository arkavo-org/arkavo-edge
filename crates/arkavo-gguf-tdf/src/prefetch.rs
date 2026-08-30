//! Decrypt-ahead worker (spec §13.3).
//!
//! Segments are independent AEAD units, so the segments the loader is about
//! to read can be decrypted on a second thread while it copies the current
//! one. Plaintext is served on exactly the same terms as the inline path:
//! both call [`decrypt_and_verify`], so a segment is handed over only after
//! its GCM tag and its manifest GMAC row agree.
//!
//! The worker holds at most `depth` decrypted segments beyond the reader
//! cache, so extra plaintext stays bounded at
//! `headerBytes + (cached_segments + depth) * maxSegment`.

// Same resolution as `segment_cache.rs`: this module is private, so
// `pub(crate)` is the real visibility; `redundant_pub_crate` wants `pub`,
// which the workspace's `unreachable_pub` lint then rejects.
#![allow(clippy::redundant_pub_crate)]

use crate::error::GgufTdfError;
use crate::read_at::decrypt_and_verify;
use opentdf::{GgufSegment, TdfEncryption, TdfMemberIndex};
use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;
use zeroize::Zeroizing;

/// One finished decrypt: the segment id and its verified plaintext, or the
/// failure that segment must be charged with when the loader asks for it.
pub(crate) type Done = (usize, Result<Zeroizing<Vec<u8>>, GgufTdfError>);

/// A worker thread decrypting segments ahead of the loader.
pub(crate) struct Prefetcher {
    /// `None` only while `Drop` closes the channel to end the worker loop.
    requests: Option<Sender<usize>>,
    results: Receiver<Done>,
    worker: Option<JoinHandle<()>>,
    in_flight: HashSet<usize>,
    /// Results received but not yet handed to the reader. Bounded by `depth`
    /// because an id leaves `in_flight` only as it lands here, and `request`
    /// counts both.
    ready: Vec<Done>,
    depth: usize,
}

impl Prefetcher {
    /// Starts a worker on `archive`.
    ///
    /// The worker opens the archive itself rather than taking a handle: a
    /// descriptor duplicated with `try_clone` shares one seek cursor, so the
    /// worker's `seek`/`read` pair would interleave with the reader's and
    /// both would decrypt the wrong bytes. Whatever this open sees is still
    /// checked against the manifest, so it can only fail closed.
    pub(crate) fn spawn(
        archive: &Path,
        encryption: TdfEncryption,
        members: TdfMemberIndex,
        segments: Vec<GgufSegment>,
        hashes: Vec<String>,
        depth: usize,
    ) -> Result<Self, GgufTdfError> {
        let mut file = File::open(archive)?;
        let (req_tx, req_rx) = channel::<usize>();
        let (done_tx, done_rx) = channel::<Done>();
        let worker = std::thread::Builder::new()
            .name("gguf-tdf-prefetch".into())
            .spawn(move || {
                // Reused across requests so a 4 MiB ciphertext copy-out is
                // allocated once, as on the inline path.
                let mut cipher = Vec::new();
                while let Ok(id) = req_rx.recv() {
                    let result = decrypt_one(
                        &mut file,
                        &encryption,
                        &members,
                        &segments,
                        &hashes,
                        id,
                        &mut cipher,
                    );
                    if done_tx.send((id, result)).is_err() {
                        break;
                    }
                }
            })
            .expect("spawn prefetch thread");

        Ok(Self {
            requests: Some(req_tx),
            results: done_rx,
            worker: Some(worker),
            in_flight: HashSet::new(),
            ready: Vec::new(),
            depth,
        })
    }

    /// Segments this prefetcher will hold ahead of the reader cache.
    pub(crate) fn depth(&self) -> usize {
        self.depth
    }

    /// Asks the worker for segment `id`, unless it is already in flight, is
    /// already decrypted and waiting, or `depth` segments are outstanding.
    pub(crate) fn request(&mut self, id: usize) {
        if self.in_flight.len() >= self.depth
            || self.in_flight.contains(&id)
            || self.ready.iter().any(|(k, _)| *k == id)
        {
            return;
        }
        if let Some(tx) = &self.requests
            && tx.send(id).is_ok()
        {
            self.in_flight.insert(id);
        }
    }

    /// Takes every result that has arrived, without blocking.
    pub(crate) fn collect(&mut self) -> Vec<Done> {
        while let Ok(done) = self.results.try_recv() {
            self.in_flight.remove(&done.0);
            self.ready.push(done);
        }
        std::mem::take(&mut self.ready)
    }

    /// Blocks until segment `id`'s result arrives, stashing any other result
    /// that arrives first. `None` means `id` was never requested, so the
    /// caller must decrypt it inline.
    pub(crate) fn wait_for(
        &mut self,
        id: usize,
    ) -> Option<Result<Zeroizing<Vec<u8>>, GgufTdfError>> {
        if let Some(pos) = self.ready.iter().position(|(k, _)| *k == id) {
            return Some(self.ready.remove(pos).1);
        }
        if !self.in_flight.contains(&id) {
            return None;
        }
        while let Ok(done) = self.results.recv() {
            self.in_flight.remove(&done.0);
            if done.0 == id {
                return Some(done.1);
            }
            self.ready.push(done);
        }
        // The worker died without answering; the caller decrypts inline and
        // so still fails closed on a genuinely bad segment.
        self.in_flight.remove(&id);
        None
    }

    /// Outstanding requests. Only the unit tests need this: it is how the
    /// `depth` cap — and so the plaintext bound — is asserted.
    #[cfg(test)]
    pub(crate) fn in_flight(&self) -> usize {
        self.in_flight.len()
    }
}

impl Drop for Prefetcher {
    fn drop(&mut self) {
        // Closing the request channel ends `recv()` in the worker loop, so
        // the join below cannot wait on a request that will never come.
        self.requests.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Reads member `id` and decrypts it into a fresh buffer.
///
/// Mirrors the inline path in `read_at`, including the ciphertext length
/// check, so a segment the worker accepts is one the inline path would also
/// have accepted.
fn decrypt_one(
    file: &mut File,
    encryption: &TdfEncryption,
    members: &TdfMemberIndex,
    segments: &[GgufSegment],
    hashes: &[String],
    id: usize,
    cipher: &mut Vec<u8>,
) -> Result<Zeroizing<Vec<u8>>, GgufTdfError> {
    let segment = segments
        .get(id)
        .ok_or_else(|| GgufTdfError::BadIndex(format!("no segment {id}")))?;
    let location = members
        .get(&segment.entry)
        .ok_or_else(|| GgufTdfError::BadIndex(format!("no member {:?}", segment.entry)))?;
    if location.size != segment.plain + crate::SEGMENT_OVERHEAD {
        return Err(GgufTdfError::TagMismatch);
    }
    let row = hashes
        .get(id)
        .ok_or_else(|| GgufTdfError::BadIndex(format!("no integrity row {id}")))?;

    cipher.clear();
    cipher.resize(location.size as usize, 0);
    file.seek(SeekFrom::Start(location.data_start))?;
    file.read_exact(cipher)?;

    // Dropped without ever reaching the caller if the verify below fails, so
    // an unauthenticated plaintext is never observable.
    let mut plain = Zeroizing::new(vec![0u8; segment.plain as usize]);
    decrypt_and_verify(encryption, cipher, &mut plain, row)?;
    Ok(plain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use opentdf::GgufSegmentKind;
    use std::io::Write;
    use std::path::Path;

    const KEY: [u8; 32] = [0x5A; 32];
    const PLAIN_LEN: usize = 64;

    /// The worker never parses GGUF: it looks a member up by name, decrypts
    /// it, and checks the manifest row. So the unit fixture is a bare zip of
    /// encrypted members rather than a whole protected model.
    struct Parts {
        _dir: tempfile::TempDir,
        path: std::path::PathBuf,
        members: TdfMemberIndex,
        segments: Vec<GgufSegment>,
        hashes: Vec<String>,
    }

    /// `count` weight members of `PLAIN_LEN` bytes each, `s/{id}` holding the
    /// byte `id` repeated. Index 0 is the header placeholder so `segments[id]`
    /// and `hashes[id]` line up with `s/{id}`.
    fn fixture(count: usize) -> Parts {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("parts.zip");
        let encryption = TdfEncryption::with_payload_key(&KEY).unwrap();

        let mut segments = vec![GgufSegment {
            id: 0,
            kind: GgufSegmentKind::Header,
            plain: 0,
            entry: crate::HEADER_ENTRY.to_string(),
        }];
        let mut hashes = vec![String::new()];

        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
        for id in 1..=count {
            let encrypted = encryption.encrypt_segment(&[id as u8; PLAIN_LEN]).unwrap();
            let entry = crate::entry_name(id as u64);
            zip.start_file(&entry, options).unwrap();
            zip.write_all(&encrypted.bytes).unwrap();
            segments.push(GgufSegment {
                id: id as u64,
                kind: GgufSegmentKind::Tensor,
                plain: PLAIN_LEN as u64,
                entry,
            });
            hashes.push(base64::engine::general_purpose::STANDARD.encode(encrypted.tag));
        }
        zip.finish().unwrap();

        Parts {
            _dir: dir,
            members: reopen_members(&path),
            path,
            segments,
            hashes,
        }
    }

    fn reopen_members(path: &Path) -> TdfMemberIndex {
        TdfMemberIndex::open(&mut File::open(path).unwrap()).unwrap()
    }

    /// Flips one ciphertext bit inside `entry`, past its 12-byte IV.
    fn corrupt(parts: &Parts, entry: &str) {
        let start = parts.members.get(entry).unwrap().data_start + 20;
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&parts.path)
            .unwrap();
        let mut byte = [0u8; 1];
        file.seek(SeekFrom::Start(start)).unwrap();
        file.read_exact(&mut byte).unwrap();
        byte[0] ^= 0x01;
        file.seek(SeekFrom::Start(start)).unwrap();
        file.write_all(&byte).unwrap();
    }

    fn spawn(parts: &Parts, depth: usize) -> Prefetcher {
        Prefetcher::spawn(
            &parts.path,
            TdfEncryption::with_payload_key(&KEY).unwrap(),
            parts.members.clone(),
            parts.segments.clone(),
            parts.hashes.clone(),
            depth,
        )
        .unwrap()
    }

    #[test]
    fn requested_segments_arrive_decrypted_and_verified() {
        let parts = fixture(3);
        let mut p = spawn(&parts, 4);
        p.request(1);
        p.request(2);

        let s1 = p.wait_for(1).unwrap().unwrap();
        let s2 = p.wait_for(2).unwrap().unwrap();
        assert_eq!(s1.as_slice(), &[1u8; PLAIN_LEN]);
        assert_eq!(s2.as_slice(), &[2u8; PLAIN_LEN]);
        assert_eq!(p.in_flight(), 0);
    }

    #[test]
    fn in_flight_is_capped_at_depth_and_duplicates_are_ignored() {
        let parts = fixture(5);
        let mut p = spawn(&parts, 2);
        for id in 1..=5 {
            p.request(id);
            p.request(id);
        }
        assert_eq!(p.in_flight(), 2, "depth caps the plaintext held ahead");
    }

    #[test]
    fn an_unrequested_id_is_not_waited_for() {
        let parts = fixture(3);
        let mut p = spawn(&parts, 4);
        assert!(
            p.wait_for(2).is_none(),
            "an id that was never requested must fall back to the inline path"
        );
    }

    #[test]
    fn a_corrupt_member_is_reported_for_that_id_only() {
        let parts = fixture(3);
        corrupt(&parts, "s/2");
        let mut p = spawn(&parts, 4);
        for id in 1..=3 {
            p.request(id);
        }

        assert!(p.wait_for(1).unwrap().is_ok());
        assert!(matches!(
            p.wait_for(2).unwrap(),
            Err(GgufTdfError::TagMismatch)
        ));
        assert!(p.wait_for(3).unwrap().is_ok());
    }

    #[test]
    fn collect_drains_without_blocking_and_leaves_nothing_in_flight() {
        let parts = fixture(3);
        let mut p = spawn(&parts, 4);
        p.request(1);
        // Waiting for 1 also proves the worker is alive; anything else it
        // finished by then is drained here rather than blocking.
        assert!(p.wait_for(1).unwrap().is_ok());
        let drained = p.collect();
        assert!(drained.is_empty());
        assert_eq!(p.in_flight(), 0);
    }

    #[test]
    fn dropping_the_prefetcher_joins_the_worker_without_hanging() {
        let parts = fixture(3);
        let mut p = spawn(&parts, 4);
        p.request(1);
        p.request(2);
        let start = std::time::Instant::now();
        drop(p);
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
    }
}
