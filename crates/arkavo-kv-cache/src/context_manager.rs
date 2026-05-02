//! Context manager for composable KV cache slots
//!
//! Manages named regions of pre-encoded KV cache on a persistent learning
//! sequence. Slots can be loaded, unloaded, and composed independently.
//! A conversation sequence is created by copying the learning sequence
//! and extending it with user input.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ContextError {
    #[error("slot '{0}' not found")]
    SlotNotFound(String),
    #[error("slot '{0}' already loaded")]
    SlotAlreadyLoaded(String),
    #[error("failed to load context from '{0}': {1}")]
    LoadFailed(String, String),
    #[error("failed to save context to '{0}': {1}")]
    SaveFailed(String, String),
}

/// Provenance tier for context slot content
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustTier {
    /// Built locally from source text
    Local,
    /// Received from a verified peer (signature-checked)
    Verified,
    /// Received from an unverified source
    Untrusted,
}

/// A named region of pre-encoded KV cache on the learning sequence
#[derive(Debug, Clone)]
pub struct ContextSlot {
    pub name: String,
    /// Start position in the learning sequence (inclusive)
    pub pos_start: i32,
    /// End position in the learning sequence (exclusive)
    pub pos_end: i32,
    pub token_count: usize,
    /// SHA-256 of the source `.bin` file
    pub source_hash: [u8; 32],
    /// Provenance of this slot's content
    pub trust: TrustTier,
}

impl ContextSlot {
    /// Number of KV cache positions occupied by this slot
    pub fn len(&self) -> i32 {
        self.pos_end - self.pos_start
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether this slot's content is from a trusted source
    pub fn is_trusted(&self) -> bool {
        matches!(self.trust, TrustTier::Local | TrustTier::Verified)
    }
}

/// Manages composable context slots on a persistent learning sequence.
///
/// Two sequences are used:
/// - `seq_learning`: Persistent sequence holding all loaded context slots
/// - `seq_conversation`: Ephemeral sequence created per conversation by
///   copying the learning sequence and appending user input
pub struct ContextManager {
    slots: Vec<ContextSlot>,
    /// Next available write position on the learning sequence
    write_head: i32,
    seq_learning: i32,
    seq_conversation: i32,
}

impl ContextManager {
    pub fn new(seq_learning: i32, seq_conversation: i32) -> Self {
        Self {
            slots: Vec::new(),
            write_head: 0,
            seq_learning,
            seq_conversation,
        }
    }

    /// Load a pre-encoded context `.bin` file as a named slot.
    ///
    /// `state_seq_load_file` restores entries at their native positions `[0, N)`
    /// and may clear the destination sequence first. When slots already exist on
    /// the learning sequence, loading directly would corrupt them. Instead,
    /// subsequent slots are staged through the conversation sequence:
    ///   1. Load onto conversation seq at `[0, N)`
    ///   2. Shift to `[write_head, write_head + N)` via `seq_add`
    ///   3. Copy to learning seq via `seq_cp`
    ///   4. Clean up conversation seq
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    pub fn load(
        &mut self,
        ctx: &arkavo_llama_cpp::LlamaContext,
        name: &str,
        path: &str,
        trust: TrustTier,
    ) -> Result<&ContextSlot, ContextError> {
        if self.slots.iter().any(|s| s.name == name) {
            return Err(ContextError::SlotAlreadyLoaded(name.to_string()));
        }

        let source_hash =
            hash_file(path).map_err(|e| ContextError::LoadFailed(path.to_string(), e))?;

        let (token_count, n) = if self.write_head == 0 {
            // First slot: load directly onto learning sequence at [0, N)
            let (tokens, _bytes) = ctx
                .state_seq_load_file(path, self.seq_learning, 128 * 1024)
                .map_err(|e| ContextError::LoadFailed(path.to_string(), e))?;
            let n = i32::try_from(tokens.len()).unwrap_or(i32::MAX);
            (tokens.len(), n)
        } else {
            // Subsequent slots: stage via conversation seq to avoid corrupting
            // existing entries on the learning seq.
            let (tokens, _bytes) = ctx
                .state_seq_load_file(path, self.seq_conversation, 128 * 1024)
                .map_err(|e| ContextError::LoadFailed(path.to_string(), e))?;
            let n = i32::try_from(tokens.len()).unwrap_or(i32::MAX);
            let mem = ctx.get_memory();

            // Shift new entries from [0, N) to [write_head, write_head+N) on staging seq
            mem.seq_add(self.seq_conversation, 0, n, self.write_head);
            // Copy from staging to learning at the target range
            mem.seq_cp(
                self.seq_conversation,
                self.seq_learning,
                self.write_head,
                self.write_head + n,
            );
            // Clean up staging area
            mem.seq_rm(self.seq_conversation, -1, -1);

            (tokens.len(), n)
        };

        let slot = ContextSlot {
            name: name.to_string(),
            pos_start: self.write_head,
            pos_end: self.write_head + n,
            token_count,
            source_hash,
            trust,
        };

        self.write_head += n;
        self.slots.push(slot);
        let idx = self.slots.len() - 1;

        Ok(&self.slots[idx])
    }

    /// Unload a named slot and compact remaining slots to fill the gap.
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    pub fn unload(
        &mut self,
        ctx: &arkavo_llama_cpp::LlamaContext,
        name: &str,
    ) -> Result<(), ContextError> {
        let idx = self
            .slots
            .iter()
            .position(|s| s.name == name)
            .ok_or_else(|| ContextError::SlotNotFound(name.to_string()))?;

        let slot = self.slots.remove(idx);
        let gap = slot.len();
        let mem = ctx.get_memory();

        // Remove the slot's range from KV cache
        mem.seq_rm(self.seq_learning, slot.pos_start, slot.pos_end);

        // Shift all entries after the removed slot backwards to compact
        if gap > 0 {
            mem.seq_add(self.seq_learning, slot.pos_end, -1, -gap);
        }

        // Update positions of remaining slots
        for s in &mut self.slots[idx..] {
            s.pos_start -= gap;
            s.pos_end -= gap;
        }

        self.write_head -= gap;
        Ok(())
    }

    /// Copy the learning sequence to the conversation sequence.
    /// Returns the write_head as the offset where new conversation tokens start.
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    pub fn begin_conversation(
        &self,
        ctx: &arkavo_llama_cpp::LlamaContext,
    ) -> Result<i32, ContextError> {
        let mem = ctx.get_memory();
        // Copy entire learning sequence to conversation
        mem.seq_cp(self.seq_learning, self.seq_conversation, -1, -1);
        Ok(self.write_head)
    }

    /// Clear the conversation sequence, preserving the learning sequence.
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    pub fn end_conversation(
        &self,
        ctx: &arkavo_llama_cpp::LlamaContext,
    ) -> Result<(), ContextError> {
        let mem = ctx.get_memory();
        mem.seq_rm(self.seq_conversation, -1, -1);
        Ok(())
    }

    /// Current write head position — where conversation tokens should start.
    pub fn conversation_offset(&self) -> i32 {
        self.write_head
    }

    /// Look up a loaded slot by name.
    pub fn get_slot(&self, name: &str) -> Option<&ContextSlot> {
        self.slots.iter().find(|s| s.name == name)
    }

    /// All loaded slots in order.
    pub fn slots(&self) -> &[ContextSlot] {
        &self.slots
    }

    /// Slots with untrusted provenance.
    pub fn untrusted_slots(&self) -> impl Iterator<Item = &ContextSlot> {
        self.slots
            .iter()
            .filter(|s| s.trust == TrustTier::Untrusted)
    }

    /// Remove all slots and reset the write head.
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    pub fn clear_all(&mut self, ctx: &arkavo_llama_cpp::LlamaContext) -> Result<(), ContextError> {
        let mem = ctx.get_memory();
        mem.seq_rm(self.seq_learning, -1, -1);
        mem.seq_rm(self.seq_conversation, -1, -1);
        self.slots.clear();
        self.write_head = 0;
        Ok(())
    }

    /// Learning sequence ID
    pub fn seq_learning(&self) -> i32 {
        self.seq_learning
    }

    /// Conversation sequence ID
    pub fn seq_conversation(&self) -> i32 {
        self.seq_conversation
    }
}

#[cfg(any(all(feature = "llama-cpp", not(target_env = "musl")), test))]
fn hash_file(path: &str) -> Result<[u8; 32], String> {
    use sha2::{Digest, Sha256};
    let data = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let hash = Sha256::digest(&data);
    Ok(hash.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("KVC-001")]
    #[test]
    fn test_context_manager_new() {
        let cm = ContextManager::new(0, 1);
        assert_eq!(cm.conversation_offset(), 0);
        assert!(cm.slots().is_empty());
        assert_eq!(cm.seq_learning(), 0);
        assert_eq!(cm.seq_conversation(), 1);
    }

    #[spec("KVC-001")]
    #[test]
    fn test_slot_len() {
        let slot = ContextSlot {
            name: "test".to_string(),
            pos_start: 10,
            pos_end: 50,
            token_count: 40,
            source_hash: [0u8; 32],
            trust: TrustTier::Local,
        };
        assert_eq!(slot.len(), 40);
        assert!(!slot.is_empty());
    }

    #[spec("KVC-001")]
    #[test]
    fn test_slot_empty() {
        let slot = ContextSlot {
            name: "empty".to_string(),
            pos_start: 0,
            pos_end: 0,
            token_count: 0,
            source_hash: [0u8; 32],
            trust: TrustTier::Local,
        };
        assert!(slot.is_empty());
        assert_eq!(slot.len(), 0);
    }

    #[spec("KVC-001")]
    #[test]
    fn test_get_slot_not_found() {
        let cm = ContextManager::new(0, 1);
        assert!(cm.get_slot("nonexistent").is_none());
    }

    #[spec("KVC-001")]
    #[test]
    fn test_context_error_display() {
        let err = ContextError::SlotNotFound("agent-role".to_string());
        assert_eq!(err.to_string(), "slot 'agent-role' not found");

        let err = ContextError::SlotAlreadyLoaded("policy".to_string());
        assert_eq!(err.to_string(), "slot 'policy' already loaded");
    }

    #[spec("KVC-004")]
    #[test]
    fn test_hash_file_nonexistent() {
        let result = hash_file("/nonexistent/file.bin");
        assert!(result.is_err());
    }

    #[spec("KVC-004")]
    #[test]
    fn test_hash_file_real() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bin");
        std::fs::write(&path, b"hello world").unwrap();
        let hash = hash_file(path.to_str().unwrap()).unwrap();
        // SHA-256 of "hello world"
        assert_ne!(hash, [0u8; 32]);
    }

    #[spec("KVC-002")]
    #[test]
    fn test_context_manager_default_sequences() {
        let cm = ContextManager::new(0, 1);
        assert_eq!(cm.seq_learning(), 0);
        assert_eq!(cm.seq_conversation(), 1);
    }

    #[spec("KVC-003")]
    #[test]
    fn test_is_trusted_tiers() {
        let make = |trust| ContextSlot {
            name: "x".to_string(),
            pos_start: 0,
            pos_end: 1,
            token_count: 1,
            source_hash: [0u8; 32],
            trust,
        };
        assert!(make(TrustTier::Local).is_trusted());
        assert!(make(TrustTier::Verified).is_trusted());
        assert!(!make(TrustTier::Untrusted).is_trusted());
    }

    #[spec("KVC-003")]
    #[test]
    fn test_untrusted_slots_filtering() {
        let mut cm = ContextManager::new(0, 1);
        cm.slots.push(ContextSlot {
            name: "local".to_string(),
            pos_start: 0,
            pos_end: 10,
            token_count: 10,
            source_hash: [0u8; 32],
            trust: TrustTier::Local,
        });
        cm.slots.push(ContextSlot {
            name: "remote".to_string(),
            pos_start: 10,
            pos_end: 20,
            token_count: 10,
            source_hash: [0u8; 32],
            trust: TrustTier::Untrusted,
        });
        let untrusted: Vec<_> = cm.untrusted_slots().collect();
        assert_eq!(untrusted.len(), 1);
        assert_eq!(untrusted[0].name, "remote");
    }
}
