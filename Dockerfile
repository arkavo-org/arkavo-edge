# Slim container image for Arkavo Edge (no local inference).
# Feature set mirrors the proven CI recipe: memory,mdns,mcp-tools,llm-remote,web-ui.
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
    --features memory,mdns,mcp-tools,llm-remote,web-ui

FROM debian:bookworm-slim

# ca-certificates: rustls needs root CAs for HTTPS to remote LLM providers.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home arkavo

COPY --from=builder /app/target/release/arkavo /usr/local/bin/arkavo

USER arkavo

# Skip interactive first-run model download; containers use remote LLMs only.
# (ARKAVO_SKIP_FIRST_RUN is added by a parallel change; harmless if unset logic absent.)
ENV ARKAVO_SKIP_FIRST_RUN=1

# AG-UI gateway default port (crates/arkavo-cli/src/commands/ui.rs).
EXPOSE 7700

ENTRYPOINT ["arkavo"]
CMD ["--help"]
