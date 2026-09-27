# GGUF-TDF

Protect on-disk GGUF weights as OpenTDF archives (`*.gguf.tdf`) and load them through llama.cpp without ever materializing a plaintext GGUF on disk or as a whole-file RAM image.

| Document | Audience |
|---|---|
| [llama.cpp loader callback handover](llama-cpp-loader-callback-handover.md) | `arkavo-llama-cpp` (funopen/`fopencookie` → public `llama_model_load_from_file_ptr`). No vendor llama.cpp patch. |
| [OpenTDF/GGUF profile design](opentdf-gguf-profile-design.md) | `opentdf-rs`, `arkavo-tdf`, wrap/unwrap, KAS |

Constraint: inference still holds weight tensors in host/GPU buffers. Extra decrypt working set is one TDF segment (default 4 MiB) plus the GGUF header.
