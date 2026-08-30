# GGUF-TDF Load Performance and PR #664 Review Follow-ups — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bring a protected `.gguf.tdf` load from ~8 s (hardware AES, single-segment cache) toward ~3 s on a 3 GB model, fix the router-init path that refuses protected models even after login, and close the two open review findings on PR #664 — all landing on branch `feature/gguf-tdf`.

**Architecture:** The reader (`crates/arkavo-gguf-tdf/src/read_at.rs`, `VirtualGguf`) serves llama.cpp's `read_at` callbacks by decrypting one 4 MiB AES-256-GCM segment at a time. llama.cpp loads tensors in model-graph order (not file order) and reads tied tensors twice, so a one-segment cache decrypts ~1.7× the model. Measured on Apple M4 Max, Gemma 4 E2B (3.68 GB, 738 segments): software AES 28 s; hardware AES 8.1 s; hardware AES + 8-segment LRU 5.5 s (835 decrypts vs 1259). The remaining ~3 s of AES sits serially on the loader thread; a decrypt-ahead worker overlaps it with the copy. Each change is independent of packing (segment layout was measured to be irrelevant).

**Tech Stack:** Rust 2024 (workspace `arkavo-edge`), `aes`/`aes-gcm` 0.8/0.10 via `opentdf-crypto`, `zeroize`, `std::thread` + `std::sync::mpsc` (no new dependencies), clap 4 for CLI, `tempfile` in tests.

**Spec:** `specifications/gguf-tdf/draft-arkavo-gguf-tdf-01.md` §13.3 (`read_at`, "Reader cache size"), §9.4, §18. Findings and measurements: https://github.com/arkavo-org/arkavo-edge/issues/667 (comments 5465898074, 5465978596).

## Global Constraints

- Branch: `feature/gguf-tdf` (PR #664). One commit per task; never force-push.
- `AGENTS.md` rules: implementation files (excluding `#[cfg(test)]`) under 400 lines; no `#[allow(dead_code)]`; no TODOs; comments explain *why*; `cargo fmt`; `cargo clippy -p <crate> --all-targets -- -D warnings` clean; ≥85 % coverage on new code; every bug fix has a regression test.
- No `--release` builds. Debug builds only (`cargo build -p arkavo` produces `target/debug/arkavo`; `-p arkavo-cli` does **not**).
- No new crates or dependencies. Prefer `std`.
- Never write plaintext weights to disk. Extra plaintext at load must stay bounded and documented as `headerBytes + N·maxSegment` with `N` stated.
- Reader must stay fail-closed: a GMAC/root-signature failure is sticky and zeroizes already-copied output (`VirtualGguf::read_at` contract, T5/T6 in `crates/arkavo-gguf-tdf/tests/roundtrip.rs`).
- Test model for timing: `~/.cache/huggingface/hub/models--unsloth--gemma-4-E2B-it-GGUF/snapshots/0314792d7f1f7e229411f620751375812bb9faf2/gemma-4-E2B-it-Q4_K_M.gguf` (plaintext, 3.68 GB). Wrap with `target/debug/arkavo model protect <path> --output <scratch>/hf/hub/models--unsloth--gemma-4-E2B-it-GGUF/snapshots/local/gemma-4-E2B-it-Q4_K_M.gguf.tdf`. Loading requires a valid identity token (`arkavo login`, already present on this machine at `~/Library/Application Support/arkavo/identity_token`) and `HF_HOME=<scratch>/hf` pointing at a hub that also contains a **plaintext** Qwen3.5-0.8B (the routing classifier needs one until Task 4 lands) and the Gemma 12B dir (first-run check). A ready scratch hub exists at `/private/tmp/claude-502/-Users-arkavo-Projects-arkavo/dcd80556-2e56-4791-87cd-0e9a5374bc01/scratchpad/hf` (aligned-packing archive at `.../scratchpad/hf-aligned`).
- Timing command: `HF_HOME=<hub> RUST_LOG=arkavo_router::provider=info target/debug/arkavo chat --model gemma-4-e2b --prompt "hi" </dev/null 2>&1 | grep -E "Loading model|Model loaded"` — the load time is the difference between the two INFO timestamps.

---

### Task 1: Hardware AES on Apple Silicon (`--cfg aes_armv8`)

**Why:** `aes` 0.8.4 uses its bitsliced software backend on aarch64 unless compiled with `--cfg aes_armv8`. Nothing sets it, so every macOS build (release included) decrypts at ~212 MB/s instead of ~1 GB/s. Measured: 28.4 s → 8.1 s for the 3 GB load. x86_64 already gets AES-NI via runtime detection.

**Files:**
- Modify: `.cargo/config.toml` (workspace root; currently has `[alias]`, `[build] pipelining = true`, `[term]`)
- Modify: `Cargo.toml` — the comment above the `[profile.dev.package.aes]` block (lines ~398–400)

**Interfaces:**
- Consumes: nothing.
- Produces: nothing code-visible. Later tasks' timing numbers assume this is in place.

- [ ] **Step 1: Add the cfg flag for aarch64 targets**

Append to `.cargo/config.toml`:

```toml
# aes 0.8 only uses the ARMv8 AES instructions when this cfg is set; without
# it every aarch64 build (release included) runs bitsliced software AES-GCM at
# ~200 MB/s, which makes a multi-GB .gguf.tdf load take tens of seconds.
# x86_64 selects AES-NI at runtime and needs nothing here.
[target.'cfg(target_arch = "aarch64")']
rustflags = ["--cfg", "aes_armv8"]
```

Note: an environment `RUSTFLAGS` overrides `target.<cfg>.rustflags` entirely. The CI jobs that set `RUSTFLAGS` (`-fuse-ld=mold`, musl static flags) are x86_64 Linux and unaffected; the macOS jobs set none.

- [ ] **Step 2: Update the Cargo.toml comment**

Replace the two comment lines above `[profile.dev.package.aes]` with:

```toml
# gguf-tdf: keep the AES-GCM path optimized in dev builds. At opt-level 0
# software AES runs ~7 MB/s and a multi-GB protected load looks like a hang.
# Hardware AES on aarch64 additionally needs `--cfg aes_armv8`
# (.cargo/config.toml).
```

- [ ] **Step 3: Rebuild and confirm the flag is applied**

Run: `cargo build -p arkavo 2>&1 | tail -1`
Expected: `Finished` with no `unexpected_cfgs` warnings.
Run: `cargo build -p arkavo -v 2>&1 | grep -m1 -o -- '--cfg aes_armv8'`
Expected: `--cfg aes_armv8` (the flag reaches rustc).

- [ ] **Step 4: Measure**

Run: `time target/debug/arkavo model protect /Users/arkavo/Library/Caches/llama.cpp/unsloth_Qwen3.5-0.8B-GGUF_Qwen3.5-0.8B-Q4_K_M.gguf --output /tmp/aes-check.gguf.tdf` then `rm /tmp/aes-check.gguf.tdf`.
Expected: under 5 s wall (was 75 s software/opt-0, ~3 s software/opt-3 is *not* expected — hardware should be ≤ 1.5 s of AES for 532 MB). Record the number for the commit message.

- [ ] **Step 5: Commit**

```bash
git add .cargo/config.toml Cargo.toml
git commit -m "Enable ARMv8 AES instructions for aarch64 builds

aes 0.8 only uses the hardware backend with --cfg aes_armv8. Without it
macOS builds decrypt .gguf.tdf segments with bitsliced software AES
(~212 MB/s); with it ~1 GB/s. 3 GB Gemma load: 28.4 s -> 8.1 s."
```

---

### Task 2: LRU of decrypted weight segments in `VirtualGguf`

**Why:** llama.cpp reads a layer's tensors in graph order across ~6 adjacent segments and reads tied tensors twice. With one cached segment that is 1259 decrypts for 738 segments. An 8-segment LRU measured 835 decrypts and 8.1 s → 5.5 s. Spec §13.3 ("Reader cache size") permits `k` cached segments with extra plaintext `headerBytes + k·maxSegment`.

**Files:**
- Create: `crates/arkavo-gguf-tdf/src/segment_cache.rs`
- Modify: `crates/arkavo-gguf-tdf/src/read_at.rs` (struct fields, `new`, `read_at`, `ensure_segment`, `Drop`)
- Modify: `crates/arkavo-gguf-tdf/src/lib.rs` (add `mod segment_cache;`, export `DEFAULT_CACHED_SEGMENTS`)
- Modify: `crates/arkavo-gguf-tdf/src/reader.rs` (`unlock` passes the default; add `unlock_with_cache`)
- Test: `crates/arkavo-gguf-tdf/src/segment_cache.rs` (unit), `crates/arkavo-gguf-tdf/tests/roundtrip.rs` (integration)

**Interfaces:**
- Produces:
  - `pub const DEFAULT_CACHED_SEGMENTS: usize = 8;` in `lib.rs`.
  - `pub(crate) struct SegmentCache` with `pub(crate) fn new(capacity: usize) -> Self`, `pub(crate) fn get(&mut self, id: usize) -> Option<&[u8]>` (marks most-recently-used), `pub(crate) fn take_slot(&mut self, plain_len: usize) -> Zeroizing<Vec<u8>>` (returns a zeroized buffer sized `plain_len`, evicting the least-recently-used entry if the cache is full), `pub(crate) fn insert(&mut self, id: usize, plain: Zeroizing<Vec<u8>>)`, `pub(crate) fn clear(&mut self)` (zeroizes every entry), `pub(crate) fn len(&self) -> usize`.
  - `VirtualGguf::segments_decrypted(&self) -> u64` (counter, for tests and §18 observability).
  - `GgufTdfArchive::unlock_with_cache(self, unwrapper: &dyn PayloadKeyUnwrapper, cached_segments: usize) -> Result<VirtualGguf, GgufTdfError>`; `unlock` calls it with `DEFAULT_CACHED_SEGMENTS`. `cached_segments == 0` is treated as 1.
- Task 3 consumes `SegmentCache::insert` / `get` and `segments_decrypted`.

- [ ] **Step 1: Write the failing unit tests for `SegmentCache`**

Create `crates/arkavo-gguf-tdf/src/segment_cache.rs` with only the tests first:

```rust
//! Bounded LRU of decrypted weight segments (spec §13.3, "Reader cache size").
//!
//! Extra plaintext is `capacity * maxSegment`. Eviction and `clear` zeroize.

use zeroize::{Zeroize, Zeroizing};

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
        assert!(slot.iter().all(|b| *b == 0), "evicted plaintext must be zeroized");
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
```

- [ ] **Step 2: Register the module and run the tests to see them fail**

Add to `crates/arkavo-gguf-tdf/src/lib.rs` next to the other `mod` lines: `mod segment_cache;` and next to `DEFAULT_MAX_SEGMENT`:

```rust
/// Decrypted weight segments a reader keeps by default (spec §13.3). Extra
/// plaintext at load is `headerBytes + DEFAULT_CACHED_SEGMENTS * maxSegment`.
pub const DEFAULT_CACHED_SEGMENTS: usize = 8;
```

Run: `cargo test -p arkavo-gguf-tdf --lib segment_cache`
Expected: compile error — `SegmentCache` not defined.

- [ ] **Step 3: Implement `SegmentCache`**

Insert above the `#[cfg(test)]` module in `segment_cache.rs`:

```rust
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
```

Run: `cargo test -p arkavo-gguf-tdf --lib segment_cache`
Expected: 6 passed.

- [ ] **Step 4: Write the failing integration test for reader behaviour**

Append to `crates/arkavo-gguf-tdf/tests/roundtrip.rs` (the file already has `build(max_segment)`, `Fixture`, `MockKas`, `unlock`):

```rust
/// Reads that revisit segments must not decrypt them again while they are
/// cached; a cache of one must (that is the draft-00 reader behaviour).
#[test]
fn cached_segments_are_not_decrypted_twice() {
    // 64 B segments: the fixture's 8 KiB token_embd alone spans >100 segments.
    let f = build(64);
    let header_bytes = 0u64; // filled below from the archive index

    let mut one = GgufTdfArchive::open(&f.archive)
        .unwrap()
        .unlock_with_cache(&f.kas, 1)
        .unwrap();
    let mut eight = GgufTdfArchive::open(&f.archive)
        .unwrap()
        .unlock_with_cache(&f.kas, 8)
        .unwrap();
    let _ = header_bytes;
    let base = one.header_bytes();
    let mut buf = [0u8; 16];

    // Touch segments 1, 2, 3, 1, 2, 3 (weight offsets base+0, +64, +128).
    for pass in 0..2 {
        for seg in 0..3u64 {
            let off = base + seg * 64;
            assert_eq!(one.read_at(off, &mut buf), 16, "pass {pass} seg {seg}");
            assert_eq!(eight.read_at(off, &mut buf), 16);
        }
    }
    assert_eq!(one.segments_decrypted(), 6, "single-entry cache re-decrypts");
    assert_eq!(eight.segments_decrypted(), 3, "LRU serves the second pass");
    assert_eq!(
        GgufTdfArchive::open(&f.archive).unwrap().unlock(&f.kas).unwrap().segments_decrypted(),
        0
    );
}

/// Bytes served through the LRU are identical to the source for every
/// offset, including reads that span several cached and uncached segments.
#[test]
fn lru_reader_serves_identical_bytes_across_segment_spans() {
    let f = build(64);
    let mut vg = GgufTdfArchive::open(&f.archive)
        .unwrap()
        .unlock_with_cache(&f.kas, 4)
        .unwrap();
    let base = vg.header_bytes();
    let total = f.source_bytes.len() as u64;
    // Forward, then backward, then a large span.
    let mut got = vec![0u8; 200];
    for start in [base, base + 64, base + 130, base + 64, base] {
        let n = vg.read_at(start, &mut got);
        let end = (start + n as u64).min(total) as usize;
        assert_eq!(&got[..n], &f.source_bytes[start as usize..end]);
    }
    let mut span = vec![0u8; 1024];
    let n = vg.read_at(base, &mut span);
    assert_eq!(&span[..n], &f.source_bytes[base as usize..base as usize + n]);
    assert!(vg.segments_decrypted() > 4, "span must have walked past the cache");
}

/// A tag failure clears the whole cache, not just the current segment.
#[test]
fn tag_failure_clears_every_cached_segment() {
    let f = build(64);
    // Flip one ciphertext byte in s/3 (third weight member) on a copy.
    let corrupted = f.archive.with_extension("corrupt.tdf");
    std::fs::copy(&f.archive, &corrupted).unwrap();
    {
        let mut file = std::fs::OpenOptions::new().read(true).write(true).open(&corrupted).unwrap();
        let members = TdfMemberIndex::open(&mut file).unwrap();
        let loc = members.get("s/3").unwrap();
        file.seek(SeekFrom::Start(loc.data_start + 12)).unwrap();
        let mut b = [0u8; 1];
        file.read_exact(&mut b).unwrap();
        file.seek(SeekFrom::Start(loc.data_start + 12)).unwrap();
        std::io::Write::write_all(&mut file, &[b[0] ^ 0x01]).unwrap();
    }
    let mut vg = GgufTdfArchive::open(&corrupted)
        .unwrap()
        .unlock_with_cache(&f.kas, 8)
        .unwrap();
    let base = vg.header_bytes();
    let mut buf = [0u8; 16];
    assert_eq!(vg.read_at(base, &mut buf), 16); // s/1 cached
    assert_eq!(vg.read_at(base + 64, &mut buf), 16); // s/2 cached
    assert_eq!(vg.read_at(base + 128, &mut buf), 0); // s/3 fails
    assert!(matches!(vg.error(), Some(GgufTdfError::TagMismatch)));
    assert_eq!(vg.cached_segments(), 0, "cache must be cleared on failure");
    assert_eq!(vg.read_at(base, &mut buf), 0, "failure is sticky");
}
```

The test needs `pub fn cached_segments(&self) -> usize` on `VirtualGguf` (count of cached weight segments) — add it in Step 6. Check the `TdfMemberIndex` API used above against `t6_flipped_ciphertext_bit_is_a_sticky_tag_mismatch` in the same file and copy its exact way of locating and flipping a member byte if it differs.

Run: `cargo test -p arkavo-gguf-tdf --test roundtrip cached`
Expected: compile error — `unlock_with_cache`, `segments_decrypted`, `cached_segments` missing.

- [ ] **Step 5: Wire the cache into `VirtualGguf`**

In `read_at.rs`:

1. Replace the fields `scratch: Zeroizing<Vec<u8>>` and `cached: Option<usize>` with `cache: crate::segment_cache::SegmentCache` and add `decrypts: u64`. Keep `cipher`, `header_plain`, `hashes`, `failed`.
2. `VirtualGguf::new(...)` gains a trailing `cached_segments: usize` parameter and initializes `cache: SegmentCache::new(cached_segments)`, `decrypts: 0`.
3. In `read_at`, the copy source becomes:

```rust
            let plaintext = if id == 0 {
                self.header_plain.as_slice()
            } else {
                // ensure_segment just inserted or promoted `id`.
                self.cache.get(id).expect("segment was just cached")
            };
```

and the two failure branches replace `self.scratch.zeroize(); self.cached = None;` with `self.cache.clear();`.

4. `ensure_segment` becomes:

```rust
    fn ensure_segment(&mut self, id: usize) -> Result<(), GgufTdfError> {
        if id == 0 || self.cache.get(id).is_some() {
            return Ok(());
        }
        let segment = self.index.segments.get(id)
            .ok_or_else(|| GgufTdfError::BadIndex(format!("no segment {id}")))?;
        let location = self.members.get(&segment.entry)
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
        let tag = self.encryption
            .decrypt_segment_into(&self.cipher, &mut plain)
            .map_err(|_| GgufTdfError::TagMismatch)?;
        self.decrypts += 1;

        let row = self.index_row(id)
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
```

(`plain` is `Zeroizing`, so the early-return paths zeroize it on drop.)

5. Add the accessors:

```rust
    /// Weight segments decrypted so far (§18 observability; tests assert on it).
    pub fn segments_decrypted(&self) -> u64 {
        self.decrypts
    }

    /// Weight segments currently held in plaintext.
    pub fn cached_segments(&self) -> usize {
        self.cache.len()
    }
```

6. `Drop`: keep the `cipher.zeroize()`; the cache's `Zeroizing` buffers clear themselves.
7. Update the module doc comment: extra plaintext is the retained header plus up to `cached_segments` weight segments (`headerBytes + k·maxSegment`, spec §13.3), and the struct doc for the removed `scratch` field.

In `reader.rs`:

```rust
    pub fn unlock(self, unwrapper: &dyn PayloadKeyUnwrapper) -> Result<VirtualGguf, GgufTdfError> {
        self.unlock_with_cache(unwrapper, crate::DEFAULT_CACHED_SEGMENTS)
    }

    /// `unlock` with an explicit number of decrypted weight segments to keep
    /// (spec §13.3). Extra plaintext is `headerBytes + cached_segments * maxSegment`.
    pub fn unlock_with_cache(
        mut self,
        unwrapper: &dyn PayloadKeyUnwrapper,
        cached_segments: usize,
    ) -> Result<VirtualGguf, GgufTdfError> {
        // existing body, passing `cached_segments` as the last argument of VirtualGguf::new
    }
```

Remove the now-unused `use zeroize::Zeroize` items in `read_at.rs` if clippy flags them.

- [ ] **Step 6: Run the full crate tests and clippy**

Run: `cargo fmt -p arkavo-gguf-tdf && cargo test -p arkavo-gguf-tdf && cargo clippy -p arkavo-gguf-tdf --all-targets -- -D warnings`
Expected: all tests pass (previous 73 + 6 unit + 3 integration), clippy clean. If `read_at.rs` non-test code exceeds 400 lines, move `index_row`/failure helpers into `segment_cache.rs` — do not shrink comments.

- [ ] **Step 7: Confirm the downstream crates still build and measure**

Run: `cargo build -p arkavo && cargo test -p arkavo-llm --features llama-cpp --test gguf_tdf_load_test`
Then the timing command from Global Constraints against the scratch hub, twice.
Expected: load ≈ 5.5 s (was ≈ 8.1 s). Record both numbers.

- [ ] **Step 8: Commit**

```bash
git add crates/arkavo-gguf-tdf
git commit -m "Keep an LRU of decrypted weight segments in VirtualGguf

llama.cpp loads a layer's tensors in graph order across a handful of
adjacent segments and reads tied tensors twice; a single cached segment
decrypted 1259 segments for a 738-segment model. Eight cached segments
(32 MiB extra plaintext, spec §13.3) cut that to ~835 and the 3 GB load
from 8.1 s to 5.5 s. Eviction and failure paths zeroize."
```

---

### Task 3: Decrypt-ahead worker (overlap AES with the loader's copy)

**Why:** After Tasks 1–2, ~3.2 s of AES-GCM still runs inline on the thread llama.cpp reads from, in front of ~2 s of copying. Segments are independent AEAD units, so a worker can decrypt the next few segments while the loader consumes the current one. Target: load ≈ 3 s.

**Files:**
- Create: `crates/arkavo-gguf-tdf/src/prefetch.rs` (worker thread, channels, in-flight bookkeeping)
- Modify: `crates/arkavo-gguf-tdf/src/read_at.rs` (`VirtualGguf` owns an `Option<Prefetcher>`; `ensure_segment` consults it before decrypting inline; after serving `id` it requests `id+1..=id+depth`)
- Modify: `crates/arkavo-gguf-tdf/src/reader.rs` (`unlock_with_cache` builds the prefetcher: needs a second `File` via `self.file.try_clone()`, a second `TdfEncryption::with_payload_key(payload_key)`, and clones of `members`, `index.segments`, `hashes`)
- Modify: `crates/arkavo-gguf-tdf/src/lib.rs` (`mod prefetch;`, `pub const DEFAULT_PREFETCH_DEPTH: usize = 4;`)
- Test: unit tests in `prefetch.rs`; integration tests appended to `tests/roundtrip.rs`

**Interfaces:**
- Consumes: `SegmentCache::{get, insert, take_slot, clear}`, `VirtualGguf::segments_decrypted`, the existing per-segment decrypt+verify sequence from Task 2 Step 5 (factor it into `pub(crate) fn decrypt_and_verify(encryption: &TdfEncryption, cipher: &[u8], plain: &mut [u8], expected_row: &str) -> Result<(), GgufTdfError>` in `read_at.rs` so the worker and the inline path share one implementation).
- Produces:
  - `pub(crate) struct Prefetcher` with `pub(crate) fn spawn(file: File, encryption: TdfEncryption, members: TdfMemberIndex, segments: Vec<SegmentInfo>, hashes: Vec<String>, depth: usize) -> Prefetcher` (where `SegmentInfo { plain: u64, entry: String }` is whatever `index.segments[i]` already is — use that type directly, cloning it), `pub(crate) fn request(&mut self, id: usize)` (no-op if already in flight or completed-but-not-collected; caps in-flight at `depth`), `pub(crate) fn collect(&mut self) -> Vec<(usize, Result<Zeroizing<Vec<u8>>, GgufTdfError>)>` (non-blocking drain), `pub(crate) fn wait_for(&mut self, id: usize) -> Option<Result<Zeroizing<Vec<u8>>, GgufTdfError>>` (blocking until that id arrives, collecting others into the returned vec's side buffer — implement as: loop `recv()` until the wanted id, stashing other results in `self.ready: Vec<...>`), `pub(crate) fn in_flight(&self) -> usize`.
  - `Drop for Prefetcher`: drop the request sender, then `join` the worker (it exits when the channel closes). Must never block forever: the worker loop is `while let Ok(id) = rx.recv()`.
  - `GgufTdfArchive::unlock_with_cache` keeps its signature; add `unlock_with_options(self, unwrapper, cached_segments: usize, prefetch_depth: usize)`; `unlock` uses `DEFAULT_CACHED_SEGMENTS` and `DEFAULT_PREFETCH_DEPTH`. `prefetch_depth == 0` disables the worker (no thread spawned).
- Extra plaintext bound becomes `headerBytes + (cached_segments + prefetch_depth) · maxSegment`; document in `read_at.rs` module docs and the `DEFAULT_PREFETCH_DEPTH` doc.

- [ ] **Step 1: Factor the shared decrypt+verify out of `ensure_segment`**

In `read_at.rs` add:

```rust
/// Decrypts one member into `plain` and checks its GMAC against the manifest
/// row. Used inline and by the prefetch worker; both must fail identically.
pub(crate) fn decrypt_and_verify(
    encryption: &TdfEncryption,
    cipher: &[u8],
    plain: &mut [u8],
    expected_row: &str,
) -> Result<(), GgufTdfError> {
    let tag = encryption
        .decrypt_segment_into(cipher, plain)
        .map_err(|_| GgufTdfError::TagMismatch)?;
    let expected = base64::engine::general_purpose::STANDARD
        .decode(expected_row)
        .map_err(|_| GgufTdfError::TagMismatch)?;
    if expected.ct_eq(&tag).unwrap_u8() != 1 {
        return Err(GgufTdfError::TagMismatch);
    }
    Ok(())
}
```

and make `ensure_segment` call it. Run `cargo test -p arkavo-gguf-tdf` — Expected: still all green (pure refactor).

- [ ] **Step 2: Write the failing unit tests for `Prefetcher`**

Create `prefetch.rs` with tests first. The tests need a real archive; build one the same way `tests/common` does but from inside the crate — the simplest is to make the unit tests use `crate::protect` with a `PreResolvedKey`-style wrapper. Check `crates/arkavo-gguf-tdf/src/key.rs` for a wrapper usable in-crate (`PreResolvedKey` implements `PayloadKeyUnwrapper`; for wrapping, write a 10-line `struct FixedKeyWrapper([u8; 32])` implementing `PayloadKeyWrapper` inside the test module that returns the key base64 as `wrapped_key`). Generate a synthetic GGUF with 3 tensors of 8 KiB using the same byte layout as `tests/common/mod.rs::synthetic_gguf` — copy that function into the test module (it is ~50 lines) rather than depending on the tests directory.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    // ... FixedKeyWrapper, synthetic_gguf copy, fn archive(max_segment) -> (TempDir, PathBuf, [u8;32])

    fn open_parts(path: &Path, key: [u8; 32]) -> (File, TdfEncryption, TdfMemberIndex, Vec<SegmentInfo>, Vec<String>) {
        // open the archive with GgufTdfArchive::open, then read the parts the
        // worker needs; expose a pub(crate) fn on GgufTdfArchive if needed
        // (e.g. `pub(crate) fn worker_parts(&self, key: &[u8]) -> ...`).
    }

    #[test]
    fn requested_segments_arrive_decrypted_and_verified() {
        let (_dir, path, key) = archive(64);
        let (file, enc, members, segs, hashes) = open_parts(&path, key);
        let mut p = Prefetcher::spawn(file, enc, members, segs, hashes, 4);
        p.request(1);
        p.request(2);
        let s1 = p.wait_for(1).unwrap().unwrap();
        let s2 = p.wait_for(2).unwrap().unwrap();
        assert_eq!(s1.len(), 64);
        assert_eq!(s2.len(), 64);
        assert_eq!(p.in_flight(), 0);
    }

    #[test]
    fn in_flight_is_capped_at_depth_and_duplicates_are_ignored() {
        let (_dir, path, key) = archive(64);
        let (file, enc, members, segs, hashes) = open_parts(&path, key);
        let mut p = Prefetcher::spawn(file, enc, members, segs, hashes, 2);
        for id in 1..=5 { p.request(id); p.request(id); }
        assert_eq!(p.in_flight(), 2);
    }

    #[test]
    fn a_corrupt_member_is_reported_for_that_id_only() {
        // flip a byte in s/2, request 1..=3, expect Ok(1), Err(TagMismatch)(2), Ok(3)
    }

    #[test]
    fn dropping_the_prefetcher_joins_the_worker_without_hanging() {
        let (_dir, path, key) = archive(64);
        let (file, enc, members, segs, hashes) = open_parts(&path, key);
        let p = Prefetcher::spawn(file, enc, members, segs, hashes, 4);
        let start = std::time::Instant::now();
        drop(p);
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
    }
}
```

Run: `cargo test -p arkavo-gguf-tdf --lib prefetch` — Expected: compile error, `Prefetcher` missing.

- [ ] **Step 3: Implement `Prefetcher`**

```rust
//! Decrypt-ahead worker (spec §13.3). Decrypts the segments the loader is
//! about to read on a second thread so AES overlaps the loader's copy.
//!
//! Extra plaintext is at most `depth` segments beyond the reader cache.

use crate::error::GgufTdfError;
use crate::read_at::decrypt_and_verify;
use opentdf::{TdfEncryption, TdfMemberIndex};
use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;
use zeroize::Zeroizing;

type Done = (usize, Result<Zeroizing<Vec<u8>>, GgufTdfError>);

pub(crate) struct Prefetcher {
    requests: Option<Sender<usize>>,
    results: Receiver<Done>,
    worker: Option<JoinHandle<()>>,
    in_flight: HashSet<usize>,
    ready: Vec<Done>,
    depth: usize,
}

impl Prefetcher {
    pub(crate) fn spawn(
        mut file: File,
        encryption: TdfEncryption,
        members: TdfMemberIndex,
        segments: Vec<opentdf::GgufSegment>, // use the crate's actual segment type
        hashes: Vec<String>,
        depth: usize,
    ) -> Self {
        let (req_tx, req_rx) = channel::<usize>();
        let (done_tx, done_rx) = channel::<Done>();
        let worker = std::thread::Builder::new()
            .name("gguf-tdf-prefetch".into())
            .spawn(move || {
                let mut cipher = Vec::new();
                while let Ok(id) = req_rx.recv() {
                    let result = decrypt_one(&mut file, &encryption, &members, &segments, &hashes, id, &mut cipher);
                    if done_tx.send((id, result)).is_err() {
                        break;
                    }
                }
            })
            .expect("spawn prefetch thread");
        Self { requests: Some(req_tx), results: done_rx, worker: Some(worker), in_flight: HashSet::new(), ready: Vec::new(), depth }
    }

    pub(crate) fn request(&mut self, id: usize) {
        if self.in_flight.len() >= self.depth || self.in_flight.contains(&id) || self.ready.iter().any(|(k, _)| *k == id) {
            return;
        }
        if let Some(tx) = &self.requests && tx.send(id).is_ok() {
            self.in_flight.insert(id);
        }
    }

    pub(crate) fn collect(&mut self) -> Vec<Done> {
        while let Ok(done) = self.results.try_recv() {
            self.in_flight.remove(&done.0);
            self.ready.push(done);
        }
        std::mem::take(&mut self.ready)
    }

    pub(crate) fn wait_for(&mut self, id: usize) -> Option<Result<Zeroizing<Vec<u8>>, GgufTdfError>> {
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
        None
    }

    pub(crate) fn in_flight(&self) -> usize {
        self.in_flight.len()
    }
}

impl Drop for Prefetcher {
    fn drop(&mut self) {
        self.requests.take(); // closes the channel; the worker loop ends
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn decrypt_one(/* as above */) -> Result<Zeroizing<Vec<u8>>, GgufTdfError> {
    // same member lookup + size check + seek/read_exact as ensure_segment,
    // then decrypt_and_verify into a fresh Zeroizing buffer of `segment.plain`.
}
```

`wait_for`'s `Some(...)` on an unknown id must return `None` so the caller falls back to inline decrypt. The `ready` vec holds decrypted plaintext; it is bounded by `depth` because `request` never exceeds it.

Run: `cargo test -p arkavo-gguf-tdf --lib prefetch` — Expected: 4 passed.

- [ ] **Step 4: Write the failing integration tests**

Append to `tests/roundtrip.rs`:

```rust
#[test]
fn prefetching_reader_serves_identical_bytes_and_decrypts_each_segment_once_on_a_sequential_pass() {
    let f = build(64);
    let mut vg = GgufTdfArchive::open(&f.archive)
        .unwrap()
        .unlock_with_options(&f.kas, 8, 4)
        .unwrap();
    let total = f.source_bytes.len();
    let mut out = vec![0u8; total];
    let mut off = 0usize;
    while off < total {
        let n = vg.read_at(off as u64, &mut out[off..(off + 100).min(total)]);
        assert!(n > 0);
        off += n;
    }
    assert_eq!(out, f.source_bytes);
    let weight_segments = (total as u64 - vg.header_bytes()).div_ceil(64);
    assert!(vg.segments_decrypted() <= weight_segments + 4, "sequential pass must not re-decrypt");
}

#[test]
fn prefetched_corrupt_segment_still_fails_closed_when_read() {
    // as in tag_failure_clears_every_cached_segment, corrupt s/3, unlock_with_options(kas, 8, 4),
    // read s/1 (triggers prefetch of 2..5), read s/3 -> 0 and sticky TagMismatch,
    // reads of s/1 afterwards return 0.
}

#[test]
fn zero_prefetch_depth_matches_the_inline_reader() {
    let f = build(64);
    let mut vg = GgufTdfArchive::open(&f.archive).unwrap().unlock_with_options(&f.kas, 1, 0).unwrap();
    let base = vg.header_bytes();
    let mut buf = [0u8; 16];
    for seg in [0u64, 1, 0] { assert_eq!(vg.read_at(base + seg * 64, &mut buf), 16); }
    assert_eq!(vg.segments_decrypted(), 3);
}
```

Run: `cargo test -p arkavo-gguf-tdf --test roundtrip prefetch` — Expected: compile error (`unlock_with_options`).

- [ ] **Step 5: Wire the prefetcher into `VirtualGguf` and `reader.rs`**

In `ensure_segment` (after the cache check, before the inline path):

```rust
        if let Some(p) = self.prefetch.as_mut() {
            for (done_id, result) in p.collect() {
                match result {
                    Ok(plain) => self.cache.insert(done_id, plain),
                    Err(e) => self.deferred_failures.push((done_id, e)),
                }
            }
            if let Some(result) = p.wait_for(id) {
                let plain = result?;
                self.decrypts += 1;
                self.cache.insert(id, plain);
                self.schedule_ahead(id);
                return Ok(());
            }
        }
        if let Some(pos) = self.deferred_failures.iter().position(|(k, _)| *k == id) {
            return Err(self.deferred_failures.remove(pos).1);
        }
```

then the inline decrypt as in Task 2, and at the end `self.schedule_ahead(id)`, where:

```rust
    fn schedule_ahead(&mut self, id: usize) {
        let Some(p) = self.prefetch.as_mut() else { return };
        let last = self.index.segments.len().saturating_sub(1);
        for next in (id + 1)..=(id + p.depth()).min(last) {
            // don't re-request what the cache already holds
            if self.cache.contains(next) { continue; }
            p.request(next);
        }
    }
```

Add `SegmentCache::contains(&self, id) -> bool` (no promotion) and `Prefetcher::depth(&self) -> usize`. `deferred_failures: Vec<(usize, GgufTdfError)>` is a new field (a prefetch error for a segment the loader never reads must not fail the load; it only fails when that segment is requested — spec §13.3 sticky-failure applies to bytes actually served). On any failure path, `self.cache.clear()` and `self.prefetch = None` (drops the worker) in addition to setting `failed`.

`decrypts` counts both inline and prefetched decrypts (the worker's result increments when collected via `wait_for`; for results collected through `collect()` also increment — simplest: increment in both loops).

In `reader.rs`, `unlock_with_options` builds the worker after `header_plain` is verified:

```rust
        let prefetch = (prefetch_depth > 0).then(|| {
            Prefetcher::spawn(
                self.file.try_clone()?,
                TdfEncryption::with_payload_key(payload_key.as_ref())?,
                self.members.clone(),
                index.segments.clone(),
                hashes.clone(),
                prefetch_depth,
            )
        }).transpose()?;
```

(`TdfMemberIndex` and the segment type may need `Clone` — check; if `TdfMemberIndex` is not `Clone`, re-open it from the cloned file with `TdfMemberIndex::open`.)

- [ ] **Step 6: Tests, clippy, size check**

Run: `cargo fmt -p arkavo-gguf-tdf && cargo test -p arkavo-gguf-tdf && cargo clippy -p arkavo-gguf-tdf --all-targets -- -D warnings && wc -l crates/arkavo-gguf-tdf/src/read_at.rs crates/arkavo-gguf-tdf/src/prefetch.rs`
Expected: all green; each file's non-test code under 400 lines.

- [ ] **Step 7: Measure and record**

Run the timing command twice against the scratch hub.
Expected: load ≈ 3 s (AES overlapped). If it is not below 5 s, sample the process (`sample <pid> 3`) and check whether `wait_for` is blocking on the loader thread — the usual cause is `depth` too small for the 1 MiB stdio refills; try `DEFAULT_PREFETCH_DEPTH = 6` before anything else, and report the numbers either way.

- [ ] **Step 8: Commit**

```bash
git add crates/arkavo-gguf-tdf
git commit -m "Decrypt segments ahead of the loader on a worker thread

Segments are independent AEAD units. A worker decrypts the next
DEFAULT_PREFETCH_DEPTH segments while llama.cpp copies the current one,
so AES no longer sits serially on the loader thread. Failures stay
sticky and are raised only when the affected segment is read. Extra
plaintext: headerBytes + (cached + depth) * maxSegment."
```

---

### Task 4: Router init must not pick a protected model for the classifier/judge

**Why:** `TaskClassifier::new()` (`crates/arkavo-router/src/classifier.rs:349-367`) and `ResponseJudge::new_local()` (`judge.rs:46-66`) call `find_any_gguf()` and then the *synchronous* `LlamaCppProvider::new`, which refuses `.gguf.tdf`. If the only Qwen3 on disk is protected, `arkavo chat` fails at router init with `GGUFTDF_KAS_DENIED` even after a successful login (#667). The classifier and judge are internal helpers; they should only ever use a plaintext model and fall through to rule-based classification otherwise.

**Files:**
- Modify: `crates/arkavo-router/src/model_discovery.rs` (`find_any_gguf`, add `find_any_plain_gguf_in`, add `find_plain_gguf_in_dir`)
- Test: `crates/arkavo-router/src/model_discovery.rs` (`protected_model_tests` module)

**Interfaces:**
- Produces: `pub async fn find_any_gguf() -> Option<PathBuf>` keeps its signature but returns only plaintext `.gguf` paths. New `fn find_any_plain_gguf_in(cache: &Path) -> Option<PathBuf>` (sync, testable with a tempdir) does the work; `find_any_gguf` calls it with `get_hf_cache_dir()`.
- Task 5 reuses the sorted-walk change made here (Step 3).

- [ ] **Step 1: Write the failing tests**

Append inside `mod protected_model_tests`:

```rust
    /// The classifier/judge load synchronously and cannot rewrap: a cache
    /// holding only protected models must yield nothing, not a `.gguf.tdf`.
    #[test]
    fn find_any_gguf_ignores_protected_models() {
        let cache = tempfile::tempdir().unwrap();
        let repo = cache.path().join("models--unsloth--Qwen3.5-0.8B-GGUF/snapshots/x");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::write(repo.join("Qwen3.5-0.8B-Q4_K_M.gguf.tdf"), b"PK\x03\x04").unwrap();

        assert_eq!(find_any_plain_gguf_in(cache.path()), None);
    }

    #[test]
    fn find_any_gguf_prefers_a_plaintext_qwen_over_other_plaintext_models() {
        let cache = tempfile::tempdir().unwrap();
        let other = cache.path().join("models--org--other/snapshots/x");
        let qwen = cache.path().join("models--unsloth--Qwen3.5-0.8B-GGUF/snapshots/x");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::create_dir_all(&qwen).unwrap();
        std::fs::write(other.join("other.gguf"), b"GGUF").unwrap();
        std::fs::write(qwen.join("Qwen3.5-0.8B-Q4_K_M.gguf"), b"GGUF").unwrap();
        // A protected sibling next to the preferred plaintext changes nothing.
        std::fs::write(qwen.join("Qwen3.5-0.8B-Q4_K_M.gguf.tdf"), b"PK\x03\x04").unwrap();

        let found = find_any_plain_gguf_in(cache.path()).unwrap();
        assert_eq!(found.file_name().unwrap(), "Qwen3.5-0.8B-Q4_K_M.gguf");
    }

    #[test]
    fn find_any_gguf_falls_back_to_any_plaintext_repo_when_preferred_ones_are_protected() {
        let cache = tempfile::tempdir().unwrap();
        let other = cache.path().join("models--org--other/snapshots/x");
        let qwen = cache.path().join("models--unsloth--Qwen3.5-0.8B-GGUF/snapshots/x");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::create_dir_all(&qwen).unwrap();
        std::fs::write(other.join("other.gguf"), b"GGUF").unwrap();
        std::fs::write(qwen.join("Qwen3.5-0.8B-Q4_K_M.gguf.tdf"), b"PK\x03\x04").unwrap();

        let found = find_any_plain_gguf_in(cache.path()).unwrap();
        assert_eq!(found.file_name().unwrap(), "other.gguf");
    }
```

Run: `cargo test -p arkavo-router --lib protected_model_tests` — Expected: compile error, `find_any_plain_gguf_in` missing. Check the exact cache dir name with `ModelChoice::LocalQwen3.cache_dir_name()` in `crates/arkavo-router/src/decision.rs` and adjust the test's `models--…` path if it differs.

- [ ] **Step 2: Implement**

Replace the body of `find_any_gguf`:

```rust
/// Scan the HuggingFace cache for any **plaintext** `.gguf`.
///
/// This is the fallback the routing classifier and response judge use. They
/// construct `LlamaCppProvider` synchronously and cannot rewrap a protected
/// model, so a `.gguf.tdf` is never a candidate here — a cache holding only
/// protected models yields `None` and the caller falls back to rule-based
/// classification instead of failing router init.
pub async fn find_any_gguf() -> Option<PathBuf> {
    let cache = get_hf_cache_dir()?;
    find_any_plain_gguf_in(&cache)
}

fn find_any_plain_gguf_in(cache: &Path) -> Option<PathBuf> {
    use crate::decision::ModelChoice;
    let preferred_repos: Vec<String> = [
        ModelChoice::LocalQwen3,
        ModelChoice::LocalMinistral3B,
        ModelChoice::LocalMinistral8B,
        ModelChoice::LocalQwen35_27B,
    ]
    .iter()
    .filter_map(ModelChoice::cache_dir_name)
    .collect();

    for repo_name in &preferred_repos {
        let repo_path = cache.join(repo_name.as_str());
        if repo_path.exists()
            && let Some(gguf) = find_plain_gguf_in_dir(&repo_path)
        {
            return Some(gguf);
        }
    }

    let mut repos: Vec<PathBuf> = std::fs::read_dir(cache)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("models--")))
        .collect();
    repos.sort();
    repos.iter().find_map(|p| find_plain_gguf_in_dir(p))
}

/// Recursively find a plaintext `.gguf`, ignoring `.gguf.tdf`.
fn find_plain_gguf_in_dir(dir: &Path) -> Option<PathBuf> {
    let mut ignored = None;
    let found = find_artifact_in_dir(dir, &mut ignored);
    found
}
```

(`find_artifact_in_dir(dir, &mut Option<PathBuf>)` from the current single-pass implementation returns the first plaintext hit and only *records* protected ones; discarding `ignored` gives the plaintext-only search.) Keep the existing `preferred_repos` comment about small models.

- [ ] **Step 3: Make the directory walk deterministic**

In `find_artifact_in_dir`, replace `for entry in entries.flatten()` with a sorted walk so results do not depend on filesystem order (Task 5's test relies on this):

```rust
    if let Ok(entries) = std::fs::read_dir(dir) {
        let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for path in paths {
```

- [ ] **Step 4: Run tests and clippy**

Run: `cargo fmt -p arkavo-router && cargo test -p arkavo-router --lib model_discovery && cargo clippy -p arkavo-router --lib -- -D warnings`
Expected: all pass (previous 12 + 3).

- [ ] **Step 5: Regression check end to end**

Stage a hub whose only Qwen is protected: `mkdir -p /tmp/hf-protected-only/hub && ln -s <scratch>/hf/hub/models--unsloth--gemma-4-E2B-it-GGUF /tmp/hf-protected-only/hub/ && ln -s ~/.cache/huggingface/hub/models--ggml-org--gemma-4-12B-it-GGUF /tmp/hf-protected-only/hub/ && mkdir -p /tmp/hf-protected-only/hub/models--unsloth--Qwen3.5-0.8B-GGUF/snapshots/local && ln -s <scratch>/hf/hub/models--unsloth--Qwen3.5-0.8B-GGUF/snapshots/local/Qwen3.5-0.8B-Q4_K_M.gguf.tdf /tmp/hf-protected-only/hub/models--unsloth--Qwen3.5-0.8B-GGUF/snapshots/local/`
Run: `cargo build -p arkavo && HF_HOME=/tmp/hf-protected-only target/debug/arkavo chat --model gemma-4-e2b --prompt "Say hi in three words." </dev/null 2>&1 | tail -3`
Expected: the model answers (router init no longer dies with `GGUFTDF_KAS_DENIED`). Before this task it fails at "Failed to initialize router". Clean up `/tmp/hf-protected-only`.

- [ ] **Step 6: Commit**

```bash
git add crates/arkavo-router/src/model_discovery.rs
git commit -m "Never hand a protected model to the routing classifier or judge

find_any_gguf feeds TaskClassifier::new and ResponseJudge::new_local,
which construct LlamaCppProvider synchronously and cannot rewrap. When
the only Qwen3 on disk was a .gguf.tdf, arkavo chat failed at router
init with GGUFTDF_KAS_DENIED even after login. The fallback now only
returns plaintext GGUFs and walks directories in sorted order."
```

---

### Task 5: Lock in cross-directory plaintext-over-protected precedence (review finding 1)

**Why:** The single-pass `find_gguf_in_dir` returns the first plaintext anywhere in the tree and only falls back to a `.gguf.tdf` if no plaintext exists — even when the protected file was encountered first. No test covers the multi-directory case, so a refactor could regress it silently.

**Files:**
- Test: `crates/arkavo-router/src/model_discovery.rs` (`protected_model_tests`)

**Interfaces:**
- Consumes: Task 4 Step 3's sorted walk (guarantees `a/` is visited before `b/`).

- [ ] **Step 1: Write the test**

```rust
    /// Precedence is tree-wide, not per directory: a protected artifact seen
    /// first must not shadow a plaintext GGUF found later in a sibling dir.
    #[test]
    fn plaintext_in_a_later_directory_beats_protected_seen_earlier() {
        let root = tempfile::tempdir().unwrap();
        let a = root.path().join("a-protected");
        let b = root.path().join("b-plain");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("model.gguf.tdf"), b"PK\x03\x04").unwrap();
        std::fs::write(b.join("model.gguf"), b"GGUF").unwrap();

        let found = find_gguf_in_dir(root.path()).expect("plaintext must be found");
        assert_eq!(found, b.join("model.gguf"));
    }

    /// And the fallback still works when the protected file is the only one,
    /// nested deeper than the directory scanned.
    #[test]
    fn protected_in_a_nested_directory_is_found_when_nothing_plain_exists() {
        let root = tempfile::tempdir().unwrap();
        let deep = root.path().join("a/snapshots/x");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("model.gguf.tdf"), b"PK\x03\x04").unwrap();

        let found = find_gguf_in_dir(root.path()).expect("protected must be found");
        assert_eq!(found, deep.join("model.gguf.tdf"));
    }
```

- [ ] **Step 2: Run**

Run: `cargo test -p arkavo-router --lib protected_model_tests`
Expected: both pass on the current implementation (this task locks behaviour in; if the first test fails, the walk is not sorted — finish Task 4 Step 3 first).

- [ ] **Step 3: Prove the test has teeth**

Temporarily change `find_gguf_in_dir` to `find_artifact_in_dir(dir, &mut protected).or(protected)` → return `protected.or(found)` order swapped, run the test, confirm it fails, revert. Do not commit the sabotage.

- [ ] **Step 4: Commit**

```bash
git add crates/arkavo-router/src/model_discovery.rs
git commit -m "Test tree-wide plaintext-over-protected discovery precedence"
```

---

### Task 6: Document the `--delete-source` trust boundary (review finding 2)

**Why:** The read-back before deleting the source is structural only (`GgufTdfArchive::open`); the wrapped payload key is not round-tripped through KAS. A wrap against a wrong KAS public key would pass and the plaintext would be gone. Users must see this at the point of decision.

**Files:**
- Modify: `crates/arkavo-cli/src/commands/model.rs` (`Protect { delete_source }` doc comment → clap help)
- Modify: `crates/arkavo-cli/src/commands/model_protect.rs` (the `removed` println and the comment on the reopen)
- Test: `crates/arkavo-cli/src/commands/model.rs` tests (or `model_protect.rs` tests, whichever already has a clap help test — grep for `render_long_help` / `debug_assert`)

- [ ] **Step 1: Write the failing test**

In the `#[cfg(test)]` module of `model.rs` (create one if absent):

```rust
    #[test]
    fn delete_source_help_states_the_verification_is_structural_only() {
        use clap::CommandFactory;
        let help = ModelArgs::command().render_long_help().to_string();
        assert!(
            help.contains("not round-tripped through KAS"),
            "help must state the trust boundary, got:\n{help}"
        );
    }
```

(`ModelArgs` is whatever the `#[derive(Args)]` struct wrapping `ModelSubcommand` is called at the top of `model.rs`.)

Run: `cargo test -p arkavo-cli --lib commands::model` — Expected: FAIL on the `contains`.

- [ ] **Step 2: Update the help text and output**

In `model.rs`:

```rust
        /// Delete the plaintext source after a successful wrap.
        ///
        /// Before deleting, the written archive is reopened and checked
        /// structurally (zip members, manifest, index). The wrapped payload
        /// key is not round-tripped through KAS, so a wrap against a wrong or
        /// stale KAS key still passes this check. Keep a backup until a load
        /// through `arkavo login` has succeeded.
        #[arg(long)]
        delete_source: bool,
```

In `model_protect.rs`, change the `removed` line to:

```rust
        println!(
            "  removed    {} (archive verified structurally; the wrapped key was not round-tripped through KAS)",
            args.path.display()
        );
```

- [ ] **Step 3: Run tests and clippy**

Run: `cargo fmt -p arkavo-cli && cargo test -p arkavo-cli --lib commands::model && cargo clippy -p arkavo-cli --lib -- -D warnings`
Expected: pass.

- [ ] **Step 4: Commit**

```bash
git add crates/arkavo-cli/src/commands/model.rs crates/arkavo-cli/src/commands/model_protect.rs
git commit -m "Say what --delete-source verifies before it deletes

The post-write check is structural; the wrapped payload key is not
round-tripped through KAS. State that in the flag's help and in the
removal line so the trust boundary is visible where the choice is made."
```

---

### Task 7: Fold results into PR #664

**Files:**
- Modify: PR #664 description (via `gh pr edit 664 --body-file`), the **Performance** table and the **Also in this PR** list.

- [ ] **Step 1: Collect the numbers** from Task 1 Step 4, Task 2 Step 7, Task 3 Step 7.
- [ ] **Step 2: Update the description**: add rows "hardware AES (`aes_armv8`)", "+ 8-segment LRU", "+ decrypt-ahead" with measured loads; replace the sentence "An 8-segment LRU measured 30 % faster and is the follow-up (#667)" with the landed state; add bullets for Tasks 1–6 under "Also in this PR"; note the extra-plaintext bound `headerBytes + (8 + 4)·maxSegment = ~58 MiB at 4 MiB segments` in **Security notes**.
- [ ] **Step 3: Comment on #667** with the final table and which items landed; leave the tied-tensor double read as the remaining open item.
- [ ] **Step 4: Push** `git push origin feature/gguf-tdf` and confirm CI run status with `gh run list --branch feature/gguf-tdf --limit 1`.

---

## Execution order

Tasks 1, 4, 5, 6 are independent of each other and of 2–3 (different crates/files) and can run in parallel. Task 2 must precede Task 3. Task 5 depends on Task 4 Step 3 (sorted walk). Task 7 runs last. All tasks commit to `feature/gguf-tdf`; parallel workers must each `git pull --rebase` before committing and must not touch files outside their task's list.

---

## Review follow-ups (PR #664 review on `9078539`, added 2026-08-30)

### Task 8: `model protect` fetches the KAS key with a hardened client

**Why:** `model_protect::run` uses `reqwest::Client::new()` (default TLS, follows redirects). The identity client uses rustls and `redirect::Policy::none()`; `validate_kas_url` only checks the initial URL, so a 3xx from `platform.arkavo.net/kas.AccessService/PublicKey` could hand the wrap to another host. Match the identity client.

**Files:**
- Modify: `crates/arkavo-cli/src/commands/model_protect.rs:56-66`
- Test: `crates/arkavo-cli/src/commands/model_protect.rs` (`#[cfg(test)]`)

**Interfaces:**
- Produces: `pub(crate) fn kas_http_client() -> reqwest::Client` in `model_protect.rs` (rustls, no redirects, 30 s timeout). Task 9 reuses it unchanged.

- [ ] **Step 1: Write the failing test**

```rust
    /// The key fetch must not follow a redirect: a 3xx from the KAS host must
    /// surface as an error, never as a key from wherever Location points.
    #[tokio::test]
    async fn kas_key_fetch_does_not_follow_redirects() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let _ = s.read(&mut buf).await;
            let _ = s
                .write_all(b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:9/elsewhere\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await;
            let _ = s.shutdown().await;
        });
        let http = kas_http_client();
        let resp = http.get(format!("http://{addr}/kas.AccessService/PublicKey")).send().await.unwrap();
        assert_eq!(resp.status().as_u16(), 302, "redirect must be returned, not followed");
    }
```

Run: `cargo test -p arkavo-cli --lib commands::model_protect` — Expected: compile error, `kas_http_client` missing. (If `tokio` is not a dev-dependency of `arkavo-cli`, check `Cargo.toml`; it is a normal dependency of the crate, which is enough.)

- [ ] **Step 2: Implement**

```rust
/// Same posture as the identity client: rustls, no redirects, bounded wait.
/// `validate_kas_url` runs on the URL we dial; following a redirect would
/// silently move the wrap to a host it never checked.
pub(crate) fn kas_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .use_rustls_tls()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("reqwest rustls client")
}
```

and replace `let http = reqwest::Client::new();` with `let http = kas_http_client();`. If `arkavo-cli`'s reqwest features do not include `rustls-tls`, add the feature to the existing `reqwest` line in `crates/arkavo-cli/Cargo.toml` (no new crate). `expect` here matches `arkavo-identity/src/session.rs`; a failure to build the client is a programming error, not a runtime condition.

- [ ] **Step 3: Tests + clippy** — `cargo test -p arkavo-cli --lib commands::model_protect && cargo clippy -p arkavo-cli --lib -- -D warnings`
- [ ] **Step 4: Commit** — `git commit -m "Fetch the KAS key without following redirects"` (body: one sentence on the identity-client parity).

---

### Task 9: `--delete-source` decrypts before it deletes

**Why:** the post-write check is `GgufTdfArchive::open` only. A structurally valid archive whose payload key was wrapped to the wrong KAS key, or whose header member is corrupt, still passes, and the only plaintext is removed. The writer holds the fresh payload key in memory, so it can prove the archive unlocks and the header authenticates before the source goes.

**Files:**
- Modify: `crates/arkavo-gguf-tdf/src/writer.rs` (`protect` gains a read-back step; `ProtectReport` gains `verified_header: bool`)
- Modify: `crates/arkavo-cli/src/commands/model_protect.rs` (use the report; drop the separate `GgufTdfArchive::open` if the writer now covers it)
- Modify: `crates/arkavo-cli/src/commands/model.rs` (`delete_source` help — see Task 6; this task supersedes Task 6's wording)
- Test: `crates/arkavo-gguf-tdf/tests/writer.rs`, `crates/arkavo-cli/src/commands/model.rs`

**Interfaces:**
- Produces: `ProtectReport { …, verified_header: bool }` — `true` when, after the rename, `GgufTdfArchive::open(dest)?.unlock(&PreResolvedKey::new(*payload_key))` succeeded (unlock already decrypts + authenticates the header and verifies the root signature). `protect` returns `Err(GgufTdfError::Crypto(..))` when the read-back fails, leaving `dest` in place for inspection and never touching the source.
- Task 6's help text becomes: "Delete the plaintext source after a successful wrap. The written archive is reopened, unlocked with the freshly generated payload key, and its header authenticated first; the KAS rewrap itself is not exercised, so a wrong KAS public key is only caught at first load. Keep a backup until a load through `arkavo login` has succeeded."

- [ ] **Step 1: Failing writer test** in `tests/writer.rs` (uses the file's existing fixture helpers; look at how it builds a source and a wrapper):

```rust
#[test]
fn protect_reads_the_archive_back_with_the_fresh_key() {
    let (dir, source) = fixture_source();          // existing helper name may differ — reuse the file's
    let dest = dir.path().join("model.gguf.tdf");
    let report = protect(&source, &dest, &MockWrapper::default(), &ProtectOptions::default()).unwrap();
    assert!(report.verified_header, "writer must prove the archive unlocks before reporting success");
}
```

- [ ] **Step 2: Implement** in `writer.rs`, after the `.partial` → `dest` rename:

```rust
    // Prove the archive we just wrote unlocks with the key we wrapped:
    // structure, header GMAC, root signature. A wrap that cannot be read
    // back must never be reported as success — --delete-source trusts this.
    GgufTdfArchive::open(dest)?
        .unlock(&PreResolvedKey::new(*payload_key))
        .map_err(|e| GgufTdfError::Crypto(format!("read-back of {} failed: {e}", dest.display())))?;
```

and set `verified_header: true` in the report. (`payload_key` is the `Zeroizing<[u8; 32]>` already in scope; `PreResolvedKey::new` takes `[u8; 32]`.)

- [ ] **Step 3: CLI** — in `model_protect.rs` replace the structural reopen block with a guard on `report.verified_header` (`bail!` if false — it cannot be false after Step 2, but the CLI must not delete on a report it did not check). Update the `removed` line: `"  removed    {} (archive read back and header authenticated; KAS rewrap not exercised)"`. Update the `delete_source` help per Interfaces and Task 6's test string accordingly (`contains("KAS rewrap itself is not exercised")`).
- [ ] **Step 4: Tests + clippy** — `cargo test -p arkavo-gguf-tdf && cargo test -p arkavo-cli --lib commands::model && cargo clippy -p arkavo-gguf-tdf -p arkavo-cli --all-targets -- -D warnings`
- [ ] **Step 5: Commit** — `git commit -m "Read the archive back with the fresh key before --delete-source removes the source"`.

---

### Task 10: Refuse absurd `headerBytes` / `maxSegment` before allocating

**Why:** `validate_index` checks alignment and multiples only; `decrypt_header` does `vec![0u8; plain_len]` and `read_manifest` does `vec![0u8; location.size]` from untrusted sizes. A hostile archive is a local allocation bomb.

**Files:**
- Modify: `crates/arkavo-gguf-tdf/src/lib.rs` (constants), `src/index.rs` (`validate_index`), `src/reader.rs` (`read_manifest`, `decrypt_header`)
- Test: `crates/arkavo-gguf-tdf/tests/index.rs`, `tests/roundtrip.rs`

**Interfaces:**
- Produces: `pub const MAX_HEADER_BYTES: u64 = 1 << 30;` (1 GiB — a tokenizer-heavy header is tens of MiB), `pub const MAX_MAX_SEGMENT: u64 = 256 << 20;`, `pub const MAX_MANIFEST_BYTES: u64 = 64 << 20;` in `lib.rs`. `validate_index` fails `BadHeader` when `header_bytes > MAX_HEADER_BYTES` and `BadMaxSegment` when `max_segment > MAX_MAX_SEGMENT`; `read_manifest` fails `BadIndex("manifest member is N bytes, over the M byte cap")` before allocating; `decrypt_header` fails `BadIndex` when the row's `segment_size` exceeds `MAX_HEADER_BYTES` or the member size exceeds `MAX_HEADER_BYTES + SEGMENT_OVERHEAD`. Writer: `plan_segments` fails `BadMaxSegment` for `max_segment > MAX_MAX_SEGMENT` (so writers cannot produce what readers refuse).

- [ ] **Step 1: Failing tests** — in `tests/index.rs`, take the `appendix_a_index()` fixture, set `index.header_bytes = MAX_HEADER_BYTES + 32` (and the header segment's `plain` to match) and assert `validate_index(..)` is `Err(BadHeader)`; set `index.max_segment = MAX_MAX_SEGMENT + 32` (aligned) and assert `Err(BadMaxSegment)`. In `tests/roundtrip.rs`, use `rewrite_manifest` (existing helper) to set `integrityInformation.segments[0].segmentSize` to `2 * MAX_HEADER_BYTES` and assert `GgufTdfArchive::open(..).unwrap().unlock(&kas)` fails with `BadIndex` **without** the test process allocating (the assertion is the error variant; if the cap is missing the test will OOM or take seconds — that is the failure mode being fixed). In `tests/packing.rs`, `plan_segments(&h, .., MAX_MAX_SEGMENT + 32)` → `GGUFTDF_BAD_MAX_SEGMENT`.
- [ ] **Step 2: Implement** the checks at the four sites; comments say why the numbers are what they are (1 GiB header: largest known tokenizer KV blocks are < 100 MiB; 256 MiB segment: 64× the default, beyond which the scratch bound stops being small).
- [ ] **Step 3: Tests + clippy** — `cargo test -p arkavo-gguf-tdf && cargo clippy -p arkavo-gguf-tdf --all-targets -- -D warnings`
- [ ] **Step 4: Commit** — `git commit -m "Cap untrusted header, segment, and manifest sizes before allocating"`. Note the caps in the spec errata later (§9.4 / §17.7) — controller task.

---

### Task 11: Fail-closed unit tests the review asked for

**Why:** reviewer-named gaps: `arkavo model list` with both artifacts present; `ModelRegistry::load` and `LlamaCppProvider::new_with_config` fail-closed on a protected path (check `crates/arkavo-llm/tests/gguf_tdf_load_test.rs` — `registry_load_refuses_a_protected_path_without_a_key` and `the_sync_constructor_refuses_a_protected_model` already exist; add only what is missing).

**Files:**
- Test: `crates/arkavo-cli/src/commands/model_list.rs` (`list_local_gguf_models` against a tempdir HF hub with `model.gguf` and `model.gguf.tdf` side by side — both listed, plaintext first), `crates/arkavo-llm/tests/gguf_tdf_load_test.rs` (add: `ModelRegistry::load` on a protected path returns `GGUFTDF_KAS_DENIED` **and** leaves `is_loaded(name)` false; `LlamaCppProvider::new_with_config` on a protected path never opens the sibling plaintext — create both files, assert the error, assert the plaintext file's mtime/atime untouched is not reliable, so instead assert `is_loaded` false and the error names the `.gguf.tdf` path).

- [ ] **Step 1:** read `list_local_gguf_models` to see how it locates the hub (`HF_HOME`?); if it reads the env var, the test must set `HF_HOME` to a tempdir — env mutation is process-global, so mark the test `#[serial]` only if the crate already uses `serial_test`; otherwise refactor a `fn list_gguf_models_in(hub: &Path)` (sync, pure) and test that.
- [ ] **Step 2:** write the tests; run `cargo test -p arkavo-cli --lib commands::model_list` and `cargo test -p arkavo-llm --features llama-cpp --test gguf_tdf_load_test`.
- [ ] **Step 3: Commit** — `git commit -m "Test dual-artifact listing and registry fail-closed paths"`.

---

### Task 12: Deterministic "latest session" pick (CI flake in `arkavo-session`)

**Why:** `conversation::tests::test_restore_last_session_with_compatibility` failed on two consecutive CI runs (`a243159e`, `9078539`) and passes locally: `restore_last_session_with_compatibility` (`crates/arkavo-session/src/conversation.rs:200-230`) takes `sessions.first()` from `memory_storage.search("conversation_session", 10, ..)` and assumes that is the newest; when two sessions are created within the same clock tick the search order is not the creation order, and the older compatible session is picked. This blocks Release Readiness for every push of this PR.

**Files:**
- Modify: `crates/arkavo-session/src/conversation.rs:214-222`
- Test: same file, `#[cfg(test)]` module

- [ ] **Step 1: Failing test** — construct two `ConversationSession`s with identical `created_at`/`updated_at` (build them by hand, then store via the same path `start_session_with_metadata` uses — read that function to see the storage call), stored in the *compatible-first* order, and assert `restore_last_session_with_compatibility` returns `None` (the incompatible one is newest by insertion). Run it 20× with `--test-threads=1` in a shell loop to confirm it fails at least once before the fix (report the count).
- [ ] **Step 2: Implement** — deserialize every search hit, choose the newest by `(updated_at, created_at)`, and on a full tie prefer the hit with the highest storage sequence if the `Memory` type exposes one (look at `memory.id` / `memory.created_at` in the `search` result), else the last hit. Replace `sessions.first()` with that selection; keep the compatibility checks unchanged.
- [ ] **Step 3:** `cargo test -p arkavo-session --lib conversation` (all pass) and the 20× loop (0 failures). Clippy clean.
- [ ] **Step 4: Commit** — `git commit -m "Pick the newest session deterministically when restoring"` with a body naming the CI runs.

---

### Task 7 (amended): fold everything into PR #664

In addition to the original Step 2: fix the Security-notes nit — "tampered or reordered → TagMismatch" becomes "a flipped ciphertext bit → `TagMismatch` (T6); an equal-size member+hash swap → `RootMismatch` at unlock (T17), which is the order bind"; add the size caps (Task 10) and the read-back-before-delete (Task 9) to Security notes; note Windows cannot load protected models and that protected `mmproj` is refused under Caveats. Reply to the review comment listing what landed per item and what was deferred.

## Execution order (amended)

1 ✓, 4+5 ✓, 2 → 3 → 10 → 9 (gguf-tdf crate, sequential), then 8, 11, 12 (independent crates, sequential dispatches), then 7.

---

## Audit follow-ups (arkavo-gguf-tdf vs opentdf-rs, 2026-08-30)

### Task 13: Hardware GHASH (`--cfg polyval_armv8`) — dispatched inline, see ledger

### Task 14: Use the library's root-signature verifier instead of a hand-rolled HMAC

**Why:** `crates/arkavo-gguf-tdf/src/reader.rs:318-352` recomputes `HMAC-SHA256(payloadKey, concat(tags))` with direct `hmac` + `sha2` dependencies. `opentdf::manifest::IntegrityInformationExt::verify_root_signature(&self, gmac_tags: &[Vec<u8>], payload_key: &[u8]) -> Result<(), String>` (already imported in `writer.rs:13`) does the same with a constant-time compare and a `Zeroizing` aggregate. The audit classified this as the crate's only unnecessary bypass. The library verifier does **not** check that each tag is 16 bytes — that check stays downstream (spec §10.4; T17 must keep failing `RootMismatch` on a bad row).

**Files:**
- Modify: `crates/arkavo-gguf-tdf/src/reader.rs` (`verify_root_signature`, imports)
- Modify: `crates/arkavo-gguf-tdf/Cargo.toml` (remove `hmac`, `sha2` if no other use — grep first; commit `Cargo.lock` if it changes)
- Test: existing `t17_equal_size_member_swap_is_caught_by_the_root_signature` and `t5/t6` in `tests/roundtrip.rs` must keep passing; add `root_signature_row_that_is_not_16_bytes_is_root_mismatch` in `tests/roundtrip.rs` using `rewrite_manifest` to set one `integrityInformation.segments[k].hash` to base64 of 15 bytes and assert `unlock` fails `RootMismatch`.

- [ ] **Step 1:** write the new test; run `cargo test -p arkavo-gguf-tdf --test roundtrip root_signature_row` — Expected: passes already (the current code checks length) — this is a lock-in test; keep it.
- [ ] **Step 2:** replace the body of `verify_root_signature` with:

```rust
    let integrity = &manifest.encryption_information.integrity_information;
    let mut tags = Vec::with_capacity(integrity.segments.len());
    for row in &integrity.segments {
        let tag = base64::engine::general_purpose::STANDARD
            .decode(&row.hash)
            .map_err(|_| GgufTdfError::RootMismatch)?;
        // The library concatenates whatever it is given; the profile requires
        // exactly one 16-byte GMAC per row (§10.4), so enforce that here.
        if tag.len() != 16 {
            return Err(GgufTdfError::RootMismatch);
        }
        tags.push(tag);
    }
    integrity
        .verify_root_signature(&tags, payload_key)
        .map_err(|_| GgufTdfError::RootMismatch)
```

  with `use opentdf::manifest::IntegrityInformationExt;` and the `hmac`/`sha2`/`ct_eq` imports removed if now unused.
- [ ] **Step 3:** `cargo test -p arkavo-gguf-tdf && cargo clippy -p arkavo-gguf-tdf --all-targets -- -D warnings`; remove the two deps from `Cargo.toml` if `grep -rn "hmac\|sha2" crates/arkavo-gguf-tdf/src` is empty; `cargo build -p arkavo-gguf-tdf` to refresh `Cargo.lock`.
- [ ] **Step 4:** commit `Verify the root signature through opentdf instead of a local HMAC` (mention opentdf-rs#100 item 2 as the API that would also retire the row decoding).

Related upstream work (not in this plan): opentdf-rs#99 (in-place segment decrypt — fix in progress on branch `fix/decrypt-segment-in-place`; when merged, bump the `opentdf` rev in `crates/arkavo-cli/Cargo.toml` and the workspace), opentdf-rs#100 (verify-side APIs).
