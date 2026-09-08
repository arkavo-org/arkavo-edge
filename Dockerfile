# Slim container image for Arkavo Edge (no local inference).
# Feature set: the `cloud` feature (arkavo/Cargo.toml), which currently
# expands to memory,mdns,mcp-tools,llm-remote,web-ui.
# llama-cpp is intentionally excluded from the feature list.
# Slim build: llama-cpp is feature-gated end to end (ui-generator, agui,
# orchestrator, server), so this image needs neither cmake nor the vendored
# llama.cpp tree.
# See docs/deploy/container.md for the rationale, gap, and known limitations.

FROM rust:1-bookworm AS builder
WORKDIR /app

# .dockerignore excludes target/, vendor/, and .git/ from the build context.
COPY . .

RUN cargo build --release -p arkavo \
    --no-default-features \
    --features cloud

FROM debian:bookworm-slim

# ca-certificates: rustls needs root CAs for HTTPS to remote LLM providers.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home arkavo

COPY --from=builder /app/target/release/arkavo /usr/local/bin/arkavo

USER arkavo

# This image has no local inference backend (no llama-cpp/snpe), so
# `startup_policy::validate_local_backend` already refuses every harness
# command before the first-run gate runs — ARKAVO_SKIP_FIRST_RUN=1 is a
# no-op here today. It is still set because this Dockerfile is the shared
# recipe for the container contract: a rebuild of this same recipe with
# `llama-cpp` added to the feature list (a local-enabled variant) inherits
# the setting, and then it matters — it keeps a TTY-less `docker run` from
# blocking on the interactive downloader. It never waives the local model
# requirement in either variant; see docs/deploy/container.md.
ENV ARKAVO_SKIP_FIRST_RUN=1

# No ARKAVO_AGUI_BIND or EXPOSE here: this image cannot run `arkavo ui` (no
# local backend to serve inference), so there is no gateway port to publish.
# A local-enabled image that adds a port mapping should set
# ARKAVO_AGUI_BIND=0.0.0.0 itself — see the opt-in in
# docs/deploy/container.md and the compose/Kubernetes examples in
# docs/deploy/self-host.md.

ENTRYPOINT ["arkavo"]
CMD ["--help"]
