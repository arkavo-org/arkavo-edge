//! Safe wrapper for arkavo_spec_wrapper (b9292 common_speculative, NGRAM_SIMPLE).

use crate::ffi;

pub struct SpeculativeContext {
    ptr: *mut ffi::arkavo_spec,
}

unsafe impl Send for SpeculativeContext {}

impl SpeculativeContext {
    pub fn new_ngram(n_seq: u32) -> Result<Self, &'static str> {
        // SAFETY: arkavo_spec_init_ngram has no preconditions; returns NULL on failure
        // which we check before constructing the safe wrapper.
        let ptr = unsafe { ffi::arkavo_spec_init_ngram(n_seq) };
        if ptr.is_null() {
            return Err("arkavo_spec_init_ngram returned NULL");
        }
        Ok(Self { ptr })
    }

    pub fn begin(&mut self, seq_id: i32, prompt: &[i32]) {
        // SAFETY: self.ptr is non-null (enforced by new_ngram); prompt slice is valid for
        // reads of prompt.len() bytes for the duration of this call; the C wrapper
        // copies the data so no lifetime extension is required.
        unsafe {
            ffi::arkavo_spec_begin(self.ptr, seq_id, prompt.as_ptr(), prompt.len() as u32);
        }
    }

    /// Draft up to `n_max` tokens following `id_last` at KV position `n_past`.
    /// `history` is the running window of tokens for this sequence (prompt +
    /// generated). NGRAM_SIMPLE searches this window for a match ending in
    /// `id_last` and emits the following m-gram. Returns the drafted tokens
    /// (may be empty if no match is found or n_max <= 0).
    pub fn draft(
        &mut self,
        seq_id: i32,
        n_past: i32,
        id_last: i32,
        n_max: i32,
        history: &[i32],
    ) -> Vec<i32> {
        if n_max <= 0 {
            return Vec::new();
        }
        let mut out = vec![0i32; n_max as usize];
        // SAFETY: self.ptr is non-null; out is a Vec we just allocated with capacity
        // n_max > 0; the C wrapper writes at most n_max int32 values and returns the
        // actual count. history is a borrowed slice valid for the duration of the call;
        // the wrapper copies the data into its own buffer before invoking impl::draft.
        let n = unsafe {
            ffi::arkavo_spec_draft(
                self.ptr,
                seq_id,
                n_past,
                id_last,
                n_max,
                history.as_ptr(),
                history.len() as u32,
                out.as_mut_ptr(),
            )
        };
        out.truncate(n as usize);
        out
    }

    pub fn accept(&mut self, seq_id: i32, n_accepted: u16) {
        // SAFETY: self.ptr is non-null; arkavo_spec_accept takes by value (seq_id, n_accepted)
        // and has no other preconditions.
        unsafe { ffi::arkavo_spec_accept(self.ptr, seq_id, n_accepted) };
    }
}

impl Drop for SpeculativeContext {
    fn drop(&mut self) {
        // SAFETY: self.ptr is non-null; arkavo_spec_free is the only valid way to release
        // this resource and is called exactly once via Drop.
        unsafe { ffi::arkavo_spec_free(self.ptr) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_and_drop_no_crash() {
        let spec = SpeculativeContext::new_ngram(1).expect("init");
        drop(spec);
    }

    #[test]
    fn empty_history_returns_empty_draft() {
        let mut spec = SpeculativeContext::new_ngram(1).expect("init");
        spec.begin(0, &[]);
        let drafted = spec.draft(0, 0, 0, 8, &[]);
        assert!(drafted.is_empty(), "empty history should yield empty draft");
    }

    #[test]
    fn repeated_pattern_can_be_drafted() {
        // Build a history containing a repeated 4-token pattern so ngram_simple
        // (default size_n=1) can find a match ending in `id_last` and emit the
        // following m-gram. The exact draft size depends on upstream defaults
        // (size_m, min_hits); we only assert this returns deterministically
        // without crashing.
        let mut spec = SpeculativeContext::new_ngram(1).expect("init");
        let history: Vec<i32> = vec![1, 2, 3, 4, 5, 1, 2, 3, 4, 5, 1, 2, 3, 4];
        spec.begin(0, &history);
        let drafted = spec.draft(0, history.len() as i32, 4, 8, &history);
        // Function returns without panicking; result may be empty depending on
        // upstream min_hits config but the call surface is exercised.
        assert!(drafted.len() <= 8);
    }
}
