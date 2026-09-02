# Container Image

The root `Dockerfile` builds a slim, no-inference Arkavo Edge image. It uses a
two-stage build: a `rust:1-bookworm` (glibc) builder stage and a
`debian:bookworm-slim` runtime stage containing only the `arkavo` binary,
CA certificates, and a non-root user.

## Build

```bash
docker build -t arkavo-edge .
```

The build runs:

```bash
cargo build --release -p arkavo \
  --no-default-features \
  --features memory,mdns,mcp-tools,llm-remote,web-ui
```

`.dockerignore` excludes `target/`, `vendor/`, and `.git/` from the build
context, so local build artifacts and the llama.cpp submodule never reach the
daemon. See the known gap in the feature-set rationale below: until
`arkavo-ui-generator`'s `llama-cpp` dependency is feature-gated, the builder
transitively compiles `arkavo-llama-cpp-sys` and needs `vendor/llama.cpp`
plus cmake, so the image does not build from a `vendor/`-less context yet.

## Run

```bash
# CLI usage
docker run --rm arkavo-edge --help

# AG-UI web gateway (default port 7700)
docker run --rm -p 7700:7700 -e GEMINI_API_KEY=$GEMINI_API_KEY arkavo-edge ui
```

## Feature-set rationale

The image ships `memory,mdns,mcp-tools,llm-remote,web-ui`:

- `llama-cpp` is **excluded** from the feature list. Local inference is
  Apple-Metal-centric and containers are expected to use remote LLM providers
  instead. **Known gap**: the C++ sys crate is still compiled transitively
  today — `arkavo-agui` depends unconditionally on `arkavo-ui-generator`,
  which declares `arkavo-llm = { features = ["llama-cpp"] }`
  (`crates/arkavo-ui-generator/Cargo.toml:19`), pulling in
  `arkavo-llama-cpp-sys`, whose `build.rs` unconditionally runs cmake against
  `vendor/llama.cpp`. This affects the CI musl build the same way (that job
  checks out submodules and has cmake available). Until that dependency is
  feature-gated (planned follow-up), a build of this Dockerfile requires
  either the gating fix or, as an interim workaround, removing `vendor/` from
  `.dockerignore` and adding `cmake`/`clang` to the builder stage.
- `llm-remote` is **included** so the binary can talk to remote providers
  (OpenAI-compatible, Gemini, Kimi, DeepSeek, xAI). This diverges deliberately
  from the musl CI variant at `.github/workflows/feature.yaml:606`, which
  builds with `memory,mdns,mcp-tools` only and therefore has no remote LLM
  support. That variant targets fully offline/embedded use; the container
  image assumes network access to an LLM API.
- `web-ui` enables the AG-UI gateway served from the container.
- `cef-ui`, `claude-agent`, `kas`, `iroh`, and the per-provider shorthands
  are off to minimize build surface and image size.

## Apple-only dependency inventory

These capabilities remain macOS-only and are unaffected by (and absent from)
the container build:

- **Metal GPU acceleration**: enabled in
  `crates/arkavo-llama-cpp-sys/build.rs` under `cfg!(target_os = "macos")`
  (`GGML_METAL`, Metal/MetalKit/MetalPerformanceShaders frameworks). Not
  compiled in this image because `llama-cpp` is disabled.
- **arkavo-mcp-macos**: declared under
  `[target.'cfg(target_os = "macos")'.dependencies]` in
  `crates/arkavo-cli/Cargo.toml` and target-gated, so it is never part of a
  Linux build.
- **Secure Enclave attestation**: `crates/arkavo-attestation/src/platform/macos.rs`
  (`SecureEnclaveAttestor`) is scaffolding — presence detection and software
  fingerprint only; true Secure Enclave signing via the Security framework is
  not yet implemented.
- **CEF runtime**: Chromium Embedded Framework support (`arkavo-cef`,
  `cef-ui` feature) is a macOS-only runtime and is disabled in this feature
  set.

## Environment variables

- `ARKAVO_SKIP_FIRST_RUN=1` — **required in containers.** Skips the
  interactive first-run flow that downloads a local model, which is
  meaningless in a no-inference image. Set by default in the Dockerfile.
  (This variable is being added by a parallel task; the Dockerfile already
  exports it, and it is a no-op until that change lands.)
- `GEMINI_API_KEY` — Gemini provider access.
- `OPENAI_API_KEY` — OpenAI-compatible provider access.
- `DEEPSEEK_API_KEY` — DeepSeek provider access.
- `ARKAVO_DEBUG=1` — general debug logging.
- `ARKAVO_DEBUG_CHAT=1` — chat/template/token debug logging.

Pass secrets with `docker run -e ...` or an orchestrator secret mechanism;
never bake them into the image.

## Known limitations

- **Unauthenticated gateway on all interfaces**: the AG-UI gateway binds
  `0.0.0.0` with no authentication (`crates/arkavo-agui/src/gateway.rs:489`).
  When publishing the port (`-p 7700:7700`), any host that can reach the
  published port can drive the agent. Run behind a reverse proxy with auth,
  or restrict to trusted networks, until gateway authentication lands.
- No local model inference: the binary always needs network access to a
  remote LLM provider.
- glibc runtime only; a fully static musl container variant can be added
  later on top of the existing musl CI build (`.github/workflows/feature.yaml`),
  keeping in mind that variant currently lacks `llm-remote`.
