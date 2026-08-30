//! Bounded LRU of decrypted weight segments (spec §13.3, "Reader cache size").
//!
//! Extra plaintext is `capacity * maxSegment`. Eviction and `clear` zeroize.

// `pub(crate)` here is the real, intended visibility (this module is private
// so nothing leaks past the crate either way); `redundant_pub_crate` wants
// `pub` instead, which `unreachable_pub` (workspace lint) then rejects. Same
// resolution as `arkavo-llm/src/llamacpp_streaming/mod.rs`.
#![allow(clippy::redundant_pub_crate)]

use zeroize::{Zeroize, Zeroizing};

/// Most-recently-used entry is last.
pub(crate) struct SegmentCache {
    entries: Vec<(usize, Zeroizing<Vec<u8>>)>,
    capacity: usize,
}

impl SegmentCache {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            entries: Vec::with_capacity(capacity.max(1)),
            capacity: capacity.max(1),
        }
    }

    /// Plaintext of segment `id`, promoting it to most recently used.
    pub(crate) fn get(&mut self, id: usize) -> Option<&[u8]> {
        let pos = self.entries.iter().position(|(k, _)| *k == id)?;
        let entry = self.entries.remove(pos);
        self.entries.push(entry);
        self.entries.last().map(|(_, plain)| plain.as_slice())
    }

    /// A zeroized buffer of `plain_len` bytes for a decrypt in progress. When
    /// the cache is full the least-recently-used entry is evicted and its
    /// buffer reused, so the plaintext it held is zeroized before reuse.
    pub(crate) fn take_slot(&mut self, plain_len: usize) -> Zeroizing<Vec<u8>> {
        let mut slot = if self.entries.len() >= self.capacity {
            let (_, mut plain) = self.entries.remove(0);
            plain.zeroize();
            plain
        } else {
            Zeroizing::new(Vec::new())
        };
        slot.clear();
        slot.resize(plain_len, 0);
        slot
    }

    pub(crate) fn insert(&mut self, id: usize, plain: Zeroizing<Vec<u8>>) {
        self.entries.retain(|(k, _)| *k != id);
        if self.entries.len() >= self.capacity {
            self.entries.remove(0);
        }
        self.entries.push((id, plain));
    }

    /// Drops every entry; `Zeroizing` clears each buffer on drop.
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(byte: u8, len: usize) -> Zeroizing<Vec<u8>> {
        Zeroizing::new(vec![byte; len])
    }

    #[test]
    fn hit_returns_the_segment_and_miss_returns_none() {
        let mut c = SegmentCache::new(2);
        c.insert(3, buf(0xAA, 4));
        assert_eq!(c.get(3).unwrap(), &[0xAA; 4]);
        assert!(c.get(4).is_none());
    }

    #[test]
    fn evicts_least_recently_used_when_full() {
        let mut c = SegmentCache::new(2);
        c.insert(1, buf(1, 4));
        c.insert(2, buf(2, 4));
        assert!(c.get(1).is_some()); // 1 is now most recent
        c.insert(3, buf(3, 4)); // evicts 2
        assert!(c.get(2).is_none());
        assert!(c.get(1).is_some());
        assert!(c.get(3).is_some());
        assert_eq!(c.len(), 2);
    }

    #[test]
    fn take_slot_reuses_an_evicted_buffer_zeroized_and_resized() {
        let mut c = SegmentCache::new(1);
        c.insert(1, buf(0xFF, 8));
        let slot = c.take_slot(4);
        assert_eq!(slot.len(), 4);
        assert!(
            slot.iter().all(|b| *b == 0),
            "evicted plaintext must be zeroized"
        );
        assert_eq!(c.len(), 0);
    }

    #[test]
    fn take_slot_when_not_full_allocates_without_evicting() {
        let mut c = SegmentCache::new(2);
        c.insert(1, buf(1, 4));
        let slot = c.take_slot(6);
        assert_eq!(slot.len(), 6);
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn clear_removes_everything() {
        let mut c = SegmentCache::new(3);
        c.insert(1, buf(1, 4));
        c.insert(2, buf(2, 4));
        c.clear();
        assert_eq!(c.len(), 0);
        assert!(c.get(1).is_none());
    }

    #[test]
    fn capacity_zero_behaves_as_one() {
        let mut c = SegmentCache::new(0);
        c.insert(1, buf(1, 4));
        c.insert(2, buf(2, 4));
        assert_eq!(c.len(), 1);
        assert!(c.get(2).is_some());
    }
}
