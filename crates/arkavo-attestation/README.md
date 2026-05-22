# arkavo-attestation

Platform and model attestation for arkavo-edge agents.

## Platform attestation

- **Platform Evidence Collection**: Securely gathers device identity, platform code, and security state.
- **Security State Detection**: Real-time detection of trusted, suspicious, or compromised system states.
- **Multiple Attestation Backends**:
  - TPM 2.0 hardware-backed quotes for Linux and Windows.
  - Secure Enclave hardware-backed attestation for macOS and iOS.
  - Software-based fingerprinting fallback for legacy or development environments.
- **Honest Reporting**: Truthful reporting of platform state to the control plane without local policy enforcement.
- **Cross-platform Support**: Unified attestation interface for macOS, Linux, Windows, and Raspberry Pi.

## Model attestation

Proves *which model* an agent process is running — the missing primitive for
agent identity. A `ModelAttestor` binds an agent to the weights it loaded and
the runtime that executes them.

- **Content-addressed weights**: `FileModelAttestor` streams a weights file
  through BLAKE3, producing a `blake3:<hex>` digest. Identical weights always
  yield the same digest; a single changed byte changes it.
- **GGUF metadata**: the GGUF header is parsed (without loading weights) to
  recover architecture, logical name, quantization, tensor count and container
  version. `safetensors` files are attested by digest and runtime alone.
- **Runtime binding**: evidence records the inference runtime (`llama.cpp`,
  `mlx`, or another) and an optional engine build identifier.
- **SPIFFE/SPIRE mapping**: `ModelEvidence::spire_selectors()` emits the exact
  `model:<key>:<value>` selectors a SPIRE workload attestor plugin would
  publish; `spiffe_id()` derives a `spiffe://…/model/<arch>/<id>/<digest>` ID.

```rust
use arkavo_attestation::{FileModelAttestor, ModelAttestor, ModelRuntime};

let evidence = FileModelAttestor::new("model.gguf", ModelRuntime::LlamaCpp)
    .with_runtime_version("llama.cpp-b4567")
    .attest()?;

for selector in evidence.spire_selectors() {
    println!("{selector}");
}
```
