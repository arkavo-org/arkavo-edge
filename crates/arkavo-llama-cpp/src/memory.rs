//! Safe wrappers for llama.cpp KV cache memory operations
//!
//! Provides sequence-level control over the KV cache: remove, copy, keep,
//! shift, and position queries. These primitives enable multi-sequence
//! patterns like persistent learning + ephemeral conversation.

use crate::ffi;

/// Safe wrapper around `llama_memory_t` — the opaque KV cache handle.
///
/// Obtained from `LlamaContext::get_memory()`. Lifetime is tied to the
/// context that created it; do not outlive the parent context.
pub struct LlamaMemory {
    ptr: ffi::llama_memory_t,
}

// SAFETY: Access serialized through the parent LlamaContext's Mutex
unsafe impl Send for LlamaMemory {}

impl LlamaMemory {
    /// Wrap a raw `llama_memory_t` pointer.
    ///
    /// # Safety
    /// Caller must ensure `ptr` is a valid, non-null memory handle
    /// obtained from `llama_get_memory`.
    pub(crate) unsafe fn from_raw(ptr: ffi::llama_memory_t) -> Self {
        Self { ptr }
    }

    /// Remove KV cache entries for `seq_id` in position range `[p0, p1)`.
    /// Pass `p0 = -1, p1 = -1` to remove the entire sequence.
    /// Returns `true` if successful.
    pub fn seq_rm(&self, seq_id: i32, p0: i32, p1: i32) -> bool {
        // SAFETY: ptr is valid for the lifetime of the parent context
        unsafe { ffi::llama_memory_seq_rm(self.ptr, seq_id, p0, p1) }
    }

    /// Copy KV cache entries from `src` to `dst` in position range `[p0, p1)`.
    /// Pass `p0 = -1, p1 = -1` to copy the entire sequence.
    pub fn seq_cp(&self, src: i32, dst: i32, p0: i32, p1: i32) {
        // SAFETY: ptr is valid for the lifetime of the parent context
        unsafe { ffi::llama_memory_seq_cp(self.ptr, src, dst, p0, p1) }
    }

    /// Remove all sequences except `seq_id` from the KV cache.
    pub fn seq_keep(&self, seq_id: i32) {
        // SAFETY: ptr is valid for the lifetime of the parent context
        unsafe { ffi::llama_memory_seq_keep(self.ptr, seq_id) }
    }

    /// Add `delta` to all positions in `[p0, p1)` for `seq_id`.
    /// Used to relocate loaded entries to a target offset.
    /// Pass `p0 = -1, p1 = -1` to shift the entire sequence.
    pub fn seq_add(&self, seq_id: i32, p0: i32, p1: i32, delta: i32) {
        // SAFETY: ptr is valid for the lifetime of the parent context
        unsafe { ffi::llama_memory_seq_add(self.ptr, seq_id, p0, p1, delta) }
    }

    /// Get the minimum position for `seq_id`. Returns -1 if empty.
    pub fn seq_pos_min(&self, seq_id: i32) -> i32 {
        // SAFETY: ptr is valid for the lifetime of the parent context
        unsafe { ffi::llama_memory_seq_pos_min(self.ptr, seq_id) }
    }

    /// Get the maximum position for `seq_id`. Returns -1 if empty.
    pub fn seq_pos_max(&self, seq_id: i32) -> i32 {
        // SAFETY: ptr is valid for the lifetime of the parent context
        unsafe { ffi::llama_memory_seq_pos_max(self.ptr, seq_id) }
    }

    /// Clear all KV cache data. If `data` is true, also clear the
    /// underlying tensor data (not just metadata).
    pub fn clear(&self, data: bool) {
        // SAFETY: ptr is valid for the lifetime of the parent context
        unsafe { ffi::llama_memory_clear(self.ptr, data) }
    }

    /// Check if the KV cache supports position shifting (SWA).
    pub fn can_shift(&self) -> bool {
        // SAFETY: ptr is valid for the lifetime of the parent context
        unsafe { ffi::llama_memory_can_shift(self.ptr) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("LLAMA-001")]
    #[test]
    fn test_llama_memory_send() {
        fn assert_send<T: Send>() {}
        assert_send::<LlamaMemory>();
    }

    #[spec("LLAMA-001")]
    #[test]
    fn test_llama_memory_size() {
        // LlamaMemory should be pointer-sized
        assert_eq!(
            std::mem::size_of::<LlamaMemory>(),
            std::mem::size_of::<*mut ()>()
        );
    }
}
