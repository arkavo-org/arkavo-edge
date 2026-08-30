# llama.cpp GGUF loader callback — handover

**Audience:** Arkavo team that owns `vendor/llama.cpp`, `arkavo-llama-cpp-sys` patches, and `arkavo-llama-cpp`.

**Companion:** [OpenTDF/GGUF profile](opentdf-gguf-profile-design.md) (zip layout, KAS, segment packing). This crate does not implement that profile.

**Status:** llama.cpp team decision — **do not** open an upstream PR and **do not** add `llama_model_load_from_callback` (or any TDF/AES/zip code) to vendored llama.cpp.

**Vendor pin at time of writing:** llama.cpp `f280b2698` (tag `b10615`). See `docs/vendor-setup.md`.

## Decision

Implement `LlamaModel::from_callback` in `arkavo-llama-cpp` as:

```text
read_at(offset, buf) -> n
        |
        v
  funopen (macOS/BSD) / fopencookie (Linux)
        |
        v
  FILE*
        |
        v
  llama_model_load_from_file_ptr(file, params)
  with params.load_mode = LLAMA_LOAD_MODE_NONE
```

That is the smallest solution: public C API only, TDF/zip/AES stay out of llama.cpp.

## Why not a llama.cpp patch

Today llama.cpp loads from a path, a `FILE*`, or synthetic metadata (`llama_model_init_from_user`). Default load mode is mmap. None of those can see TDF ciphertext, but a cookie `FILE*` **is** a `FILE*`.

`llama_model_load_from_file_ptr` already exists. On Unix, `llama_file(FILE*)` uses `fread`/`fseeko` (`fd == -1`). Cookie streams have no usable `fileno`; mmap would fail. Forcing `LLAMA_LOAD_MODE_NONE` uses the fread path.

Do **not** use `llama_model_init_from_user`: `weights_map` is not filled from the GGUF tensor directory; tensors default to F32; `set_tensor_data` skips the GPU async upload loop in `load_all_data`.

## What `arkavo-llama-cpp` owns

- `LlamaModel::from_callback`
- Cookie `FILE*` (seek/read/size) wrapping a Rust `read_at`
- GPU-then-CPU fallback, same as `from_file_with_options`, with `load_mode = NONE`
- Unit tests for cookie I/O and reject-non-GGUF

## What this crate must not own

- OpenTDF, zip, AES-GCM, KAS, `.gguf.tdf` parsing
- Payload-key lifetime
- Segment packing / hybrid manifest
- `arkavo-llm` path detection

The callback only copies **virtual linear GGUF** bytes. Byte `0` is `GGUF` magic. Tensor data sits at `gguf_get_data_offset + tensor_offset`, identical to the source `.gguf` before wrapping.

## Contract for the TDF layer (caller)

```rust
impl LlamaModel {
    /// `read_at(offset, buf)` copies virtual-GGUF bytes into `buf` and returns
    /// how many were copied (0 on EOF or error). `virtual_size` is that
    /// linear GGUF's length in bytes (not the zip length).
    pub fn from_callback<F>(virtual_size: u64, read_at: F) -> Result<Self, String>
    where
        F: FnMut(u64, &mut [u8]) -> usize;
}
```

Caller guarantees:

- Random-access reads over `[0, virtual_size)`.
- `virtual_size` equals the original GGUF file size (header + aligned tensor blob).
- Short read (`< buf.len()`) is EOF or error.
- Concurrent `read_at` is not required (`load_all_data` reads sequentially).
- Cache the current decrypted TDF segment. A cookie stream has no fd, so libc cannot size its buffer from `st_blksize`; macOS/BSD fall back to `BUFSIZ` (1 KiB) refills unless the crate calls `setvbuf(fp, NULL, _IOFBF, >= 1 MiB)` before handing the `FILE*` to llama.cpp. Either way `read_at` may be called far more often than once per tensor.

llama.cpp / cookie layer guarantees:

- `SEEK_END` + `ftell` returns `virtual_size` (`llama_file(FILE*)` measures size that way).
- `owns_fp = false` in llama.cpp — Rust `fclose`s after load returns.
- Same `FILE*` can be reused for CPU retry; a new `llama_file(FILE*)` rewinds (`END` then `SET 0`).
- `LlamaModel` does not retain the `FILE*` or the closure.

## Cookie implementation notes

| Target | API |
|---|---|
| macOS / iOS / BSD / Android | `funopen` |
| Linux glibc | `fopencookie` |
| musl | llama.cpp already stubbed in this crate |
| Windows | llama-cpp not in default features; `from_callback` returns an error if compiled |

`llama_file(FILE*)` constructor:

```text
seek(0, SEEK_END);
size = tell();
seek(0, SEEK_SET);
```

Cookie seek must implement `SEEK_SET`, `SEEK_CUR`, and `SEEK_END`. Read copies from `read_at(pos, buf)` and advances `pos`.

Do not `mmap`. `params.load_mode = LLAMA_LOAD_MODE_NONE` on both GPU and CPU attempts.

Call `setvbuf(fp, NULL, _IOFBF, n)` with `n >= 1 MiB` right after `funopen`/`fopencookie` so large `fread`s refill in big chunks instead of `BUFSIZ`. Add a test that counts `read_at` invocations for a fixture read.

## Tests

- Cookie `SEEK_END` / `fread` / `SEEK_SET` against an in-memory buffer equals the source bytes.
- `virtual_size == 0` → `Err`, no successful model pointer.
- Non-GGUF bytes → `from_callback` returns `Err`.
- Optional: `ARKAVO_TEST_MODEL` GGUF loaded via `from_callback` wrapping `std::fs` matches `from_file` on `n_vocab()`.

No `.gguf.tdf` fixture in this crate.

## Acceptance

- No diff under `vendor/llama.cpp` or `arkavo-llama-cpp-sys/patches/`.
- `from_callback` calls `llama_model_load_from_file_ptr` only (not `llama_load_model_from_file`, not `init_from_user`).
- Existing `from_file` mmap path unchanged.
- `cargo test -p arkavo-llama-cpp` passes.

## Follow-ups (not this crate)

- TDF `read_at` over `gguf-tdf/1` segments + KAS (`arkavo-tdf`).
- `arkavo-llm` uses `from_callback` when the path ends in `.gguf.tdf`.
- mmproj: `MtmdContext::from_file` still takes a filesystem path. Vision on `.gguf.tdf` is later.
