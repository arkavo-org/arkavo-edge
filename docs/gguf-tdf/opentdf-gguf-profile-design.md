# OpenTDF / GGUF profile design (`gguf-tdf/1`)

**Status:** Proposed  
**Date:** 2026-08-28  
**Companion:** [llama.cpp loader callback handover](llama-cpp-loader-callback-handover.md)

How a GGUF becomes a `.gguf.tdf`, how KAS binds policy, and how Arkavo decrypts **one segment at a time** into `LlamaModel::from_callback`.

Arkavo profile of OpenTDF. A full `otdfctl decrypt` is **not** required to emit a vanilla `.gguf`.

## Goals

- At rest, the only on-disk model artifact is ciphertext plus the TDF manifest.
- At load, plaintext exists only as: GGUF header, one TDF segment scratch (default 4 MiB), and ggml/GPU weight buffers.
- One payload key, one KAS policy, AES-256-GCM per segment, GMAC root signature.
- Variable segment sizes aligned to GGUF layout.
- llama.cpp sees a virtual linear GGUF via a cookie `FILE*` (`funopen`/`fopencookie` → `llama_model_load_from_file_ptr` + `LLAMA_LOAD_MODE_NONE`). No llama.cpp patch.

## Non-goals

- Encrypting weights in VRAM after load.
- NanoTDF or ZTDF-JSON (`TdfJsonRpc` / current `OpenTdfService`).
- Plaintext temp `.gguf` (named or whole-file memfd).
- Split-GGUF in v1.
- Changing the KAS wire protocol.
- Upstream llama.cpp PRs.

## Why not current opentdf-rs file APIs

Pinned `opentdf-rs` (`62b1fdf`) `Tdf::encrypt_file` / `decrypt_file` `std::fs::read` the whole file. Segments are a fixed 2 MiB cut. `arkavo-tdf` `OpenTdfService` uses inline base64 ZTDF-JSON; `encrypt_stream` is `read_to_end`.

Those cannot wrap 8–12B GGUFs. This profile adds GGUF-aware, multi-entry, per-segment APIs on the same AES-GCM primitive.

## Archive layout

`model.gguf.tdf` is ZIP with **Stored** compression (no deflate).

```
model.gguf.tdf
├── 0.manifest.json      # OpenTDF manifest + gguf hybrid index (plaintext JSON)
├── header               # segment 0: encrypted GGUF header (magic..data_offset)
└── s/1 … s/{n}          # encrypted tensor segments, one zip member each
```

No concatenated `0.payload`. Each member is one OpenTDF segment: `[12-byte IV][ciphertext || 16-byte tag]`. On-disk length = `segmentSize + 28`.

### Virtual GGUF

What `read_at` presents to llama.cpp is byte-identical to the source `.gguf`:

| Virtual range | Source |
|---|---|
| `[0, headerBytes)` | Decrypt `header` |
| `[headerBytes, virtualSize)` | Decrypt covering `s/{id}`, copy overlap |

`virtualSize` = original GGUF file length, including 32-byte tensor alignment padding.

## Manifest

Standard OpenTDF `TdfManifest` plus a `gguf` object.

- `payload.url`: `"header"` (not `0.payload`)
- `payload.mimeType`: `"application/x-gguf+tdf"`
- `payload.protocol`: `"zip"`
- `method.algorithm`: `"AES-256-GCM"`, `isStreamable`: true, `iv` empty
- `integrityInformation.segments[]`: one entry per zip member, **variable** `segmentSize` / `encryptedSegmentSize`
- Readers must use per-segment sizes, not `segmentSizeDefault` (header and packed tails differ)

Hybrid index (shape):

```json
{
  "gguf": {
    "profile": "gguf-tdf/1",
    "alignment": 32,
    "headerBytes": 123456,
    "virtualSize": 4294967296,
    "maxSegment": 4194304,
    "tensors": [
      {
        "name": "token_embd.weight",
        "offset": 123456,
        "size": 16777216,
        "segments": [1, 5]
      }
    ],
    "segments": [
      { "id": 0, "kind": "header", "plain": 123456, "entry": "header" },
      { "id": 1, "kind": "tensor", "plain": 4194304, "entry": "s/1" }
    ]
  }
}
```

`tensors[].segments` is a half-open index range. `kind` is `header` | `tensor` | `pack`. Tensor names in the index are not secret; header bytes (including tokenizer) are encrypted.

## Segment packing

`ALIGN = general.alignment` or 32. `maxSegment = 4 MiB` (multiple of `ALIGN`). `headerBytes = gguf_get_data_offset`.

- Segment 0: virtual `[0, headerBytes)` → `header`
- Pack consecutive small tensors (and alignment padding) until `maxSegment`
- Split large tensors on `ALIGN` at `maxSegment`
- `sum(plain) == virtualSize`

Peak decrypt scratch = `max(headerBytes, maxSegment)`. Tokenizer KV in the header may be tens of MiB; accepted.

## Wrap flow

`arkavo-tdf` + CLI (`arkavo model protect`). Not MCP `tdf_encrypt` (ZTDF-JSON).

1. Parse GGUF header; refuse non-GGUF.
2. Build segment plan without loading weights.
3. Generate payload key; wrap with KAS; bind policy (`PolicyBuilder` / `arkavo_attrs`).
4. Stream each virtual range from the source `.gguf` (seek+read that slice only); write zip member; record GMAC.
5. Write `0.manifest.json` last (root signature over all tags).
6. Do not delete the source `.gguf` unless the caller opts in.

## Unwrap-on-load flow

1. Path ends with `.gguf.tdf` or zip + `gguf.profile == "gguf-tdf/1"`.
2. Parse `0.manifest.json`; fail closed on unknown profile.
3. KAS rewrap → 32-byte payload key. Fail closed. No sibling-`.gguf` fallback.
4. `LlamaModel::from_callback(virtualSize, read_at)` with userdata: zip handle (mmap of **ciphertext** zip is fine), segment table, payload key, scratch of `maxSegment + 28` / `maxSegment`.
5. `read_at`: binary-search covering segments; decrypt member; copy overlap; zeroize scratch. Cache the current segment (stdio may read 4 KiB at a time).
6. On drop: zeroize key; drop zip; no temp files.

```text
fn read_at(ud, dst, offset, len) -> usize:
    clip to virtual_size
    while bytes remain:
        seg = segment_covering(offset)
        decrypt_segment(seg) into plain_scratch   # AES-GCM tag check
        copy overlap into dst
        zeroize used scratch
```

## opentdf-rs work

- Variable-length segment encrypt (pre-cut slices, not `chunks(2MB)`).
- `decrypt_entry(name, payload_key, dest)` bounded to `segmentSize`.
- Multi-member zip writer (`header`, `s/{id}`).
- Optional `gguf` field on `TdfManifest` (must not break SwarmKit JSON manifests).
- rustls only; `kas` / `kas-client` unchanged.

## Edge crate split

| Crate | Responsibility |
|---|---|
| `opentdf-rs` | Variable segments, multi-entry zip, `decrypt_entry`, `gguf` JSON |
| `arkavo-tdf` | `gguf-tdf/1` packer + TDF `read_at` + KAS |
| `arkavo-llama-cpp` | `LlamaModel::from_callback` (cookie `FILE*` → `llama_model_load_from_file_ptr`) |
| `arkavo-llm` | `.gguf.tdf` → callback load |
| `arkavo-router` discovery | Recognize `.gguf.tdf` |
| CLI | `arkavo model protect`; `--model foo.gguf.tdf` |

Do not add TDF to `arkavo-llama-cpp`. `arkavo-config-encryption` stays on small in-memory zip TDF for config bundles.

## Policy

Reuse Arkavo FQNs, e.g. `attr/data/clearance` and `attr/model/tier`. Offline: in-environment KAS (`SECURITY.md`). Manifest `gguf` index is plaintext (arch, tensor names, sizes, KAS URL).

## Errors

| Condition | Behavior |
|---|---|
| Not zip / missing manifest | Error |
| Unknown `gguf.profile` | Error |
| KAS deny / unreachable | Error; no plaintext fallback |
| Segment tag mismatch | Error; zeroize scratch |
| `virtualSize` ≠ sum of `plain` | Error at open |
| mmproj `.gguf.tdf` | Error until mtmd FILE* follow-up |

## Testing

- Pack a tiny/synthetic GGUF; zip has `0.manifest.json`, `header`, `s/1`; `virtualSize` equals source length.
- `read_at(0, 4)` is `GGUF` after mock KAS.
- Tensor-range `read_at` matches source GGUF bytes.
- `maxSegment = 4096` on a larger tensor → multiple `s/*`; reads still match.
- Wrong key / flipped bit → decrypt error.
- Discovery finds `.gguf.tdf`.
- Integration: Gemma 270M `.gguf.tdf` vs source `.gguf` first-token logits at temp 0.

No production KAS in unit tests.

## Phasing

**P0** — `LlamaModel::from_callback` (this handover; in progress in `arkavo-llama-cpp`).  
**P0** — opentdf-rs variable segments + multi-entry zip.  
**P1** — packer + TDF `read_at` + CLI protect (tiny GGUF, mock KAS).  
**P1** — `arkavo-llm` path detect.  
**P2** — discovery, first-run wrap, docs.  
**P3** — split GGUF, mtmd, optional root-signature-on-load.

## Decisions locked

- No plaintext temp GGUF.
- Custom multi-entry TDF; not “decrypt to vanilla GGUF.”
- No llama.cpp patch; cookie `FILE*` + `LLAMA_LOAD_MODE_NONE`.
- Default `maxSegment` 4 MiB, multiple of 32.
- Encrypted header as its own segment.
- Fail closed on KAS/policy/tag errors.
