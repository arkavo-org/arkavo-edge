# Hello World Agent

<!-- ARKAVO-CAPABILITY: llm-core -->
> **Specs**: [6 scenarios](../../specs/arkavo-edge/llm-core.spec.yaml)
> **Browse**: `cargo xtask capabilities llm-core`
<!-- /ARKAVO-CAPABILITY -->

Your first Arkavo response in 5 minutes.

## What You'll Learn

- How to send a one-shot prompt with `arkavo chat`
- What a single-role SwarmKit kit looks like
- How to run that kit as a long-lived agent

## Prerequisites

An `arkavo` binary: an installed one, or a source build. `run.sh` uses `$BINARY` if set, then the source build (`target/debug/arkavo`, then `target/release/arkavo`), then `arkavo` on `PATH`.

```bash
# Only when running from source (from repo root)
cargo build
```

## Quick Start

```bash
# From this directory
./run.sh
```

The script sends one greeting prompt, prints the response, and exits.

```bash
# Pick the model
./run.sh --model ministral-3b
```

## What's Happening

`run.sh` runs one command from this directory:

```bash
arkavo chat --prompt "Hello! Please introduce yourself briefly. What are you and what can you help with?"
```

- `arkavo chat --prompt` is a one-shot query: it answers and exits. No agent keeps running, and there is nothing to stop.
- The model is the router's default unless you pass `--model`.
- `arkavo chat` discovers `hello-agent.swarmkit.yaml` in the working directory and applies its `runtime` block (here, the cloud policy). It does not take the role's model or instructions from the kit.
- `run.sh` does not read `tasks.json`.

On the first run with no local models downloaded, `arkavo` offers to download them and exits when the download finishes. Run `./run.sh` again to get the greeting. See the [runbook](RUNBOOK.md) for sizes.

## Run the Kit as an Agent

To run the role defined in the kit, with its model (`ministral` `3B`) and instructions, start it as an agent:

```bash
arkavo model download ministral-3b   # once, if the model is not cached
arkavo agent -c hello-agent.swarmkit.yaml -v
```

This process keeps running and serves requests until you stop it with `Ctrl+C`. From another terminal, talk to it by its role id:

```bash
arkavo chat --agent-id agent --prompt "Hello!"
```

## Files

| File | Purpose |
|------|---------|
| `hello-agent.swarmkit.yaml` | Single-role kit (role id, model, instructions, runtime settings) |
| `tasks.json` | The same greeting as a demo scenario, read by `../demo.sh` and `../transition.sh` |
| `run.sh` | One-shot chat script |

## Next Steps

- Try [code-agent-claude](../code-agent-claude/README.md), [code-agent-gemini](../code-agent-gemini/README.md), or [secure-agent](../secure-agent/README.md) to explore different LLM backends and policies
- Try [software-development-simple](../software-development-simple/README.md) to see agents collaborate
- Read [CONCEPTS.md](../CONCEPTS.md) for deeper understanding
