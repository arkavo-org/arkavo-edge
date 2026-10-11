# Swarm Image Assets

A person asks any agent in a swarm for images, such as game sprites or textures. The agent whose Mac can generate them does so with a local Qwen-Image model, and the PNGs travel back over the Iroh data plane. No cloud inference service is used at any step. Behavior is specified in `specs/arkavo-edge/image-gen.spec.yaml` and `specs/arkavo-edge/swarm-assets.spec.yaml`.

## Who generates

An agent generates images when its host is macOS on Apple Silicon running the native arm64 binary, and its memory budget holds at least one catalog entry's smallest weight set. The budget is unified memory, less the chat model the agent keeps resident, less 4 GiB for the OS.

| Unified memory | Resident chat model | Budget | Weight set (both entries) |
|---|---|---|---|
| 64 GiB | Gemma 4 12B | 53.3 GiB | XL (BF16 diffusion, F16 text encoder) |
| 32 GiB (M6 Mac mini) | Gemma 4 12B | 21.3 GiB | L (Q8_0 diffusion, Q8_0 text encoder) |
| 24 GiB | Gemma 4 12B | 13.3 GiB | S (Q4_K diffusion, Q4_K_M text encoder) |
| 16 GiB | any | at most 9.2 GiB | none: asks a member instead |

Every other agent with a data plane offers the same `generate_image` tool and sends the work to a member that reports the capability.

The catalog has two entries:

- `qwen-image-2.1-turbo` (alias `qwen-image`, the default): 8 steps, 1024x1024 by default, for drafts and most game assets.
- `qwen-image-2.1` (alias `qwen-image-quality`): 40 steps, 2048x2048 by default, optional true CFG with a negative prompt.

Both are under the Qwen Research License; every delivery prints the license line. Whether it permits a given use is the requester's decision.

## Asking for assets

From any machine on the swarm, talk to any agent:

```bash
arkavo chat --agent-id <agent> --prompt "Make a transparent 512x512 sprite of a gold coin and a tileable 1024x1024 grass texture for my game"
```

The agent's model calls `generate_image` once per asset. Progress (weights fetched, then step s of S) is printed while it works. The answer's text is printed, then the files are fetched over Iroh, checked against the manifest (size, SHA-256, PNG dimensions), and saved:

```
Assets saved to arkavo-assets/<task id>:
  arkavo-assets/<task id>/coin_000.png  412311 bytes, 512x512, alpha
  Generated locally with qwen-image-2.1-turbo (stable-diffusion.cpp master-956-1b0ba10); license: ...
```

`--out <dir>` saves elsewhere. An existing file is never overwritten. Polling continues while the task reports progress, and stops after 5 minutes with no change, saying how to read the result later.

An agent can also be sent a structured request directly: a `message/send` whose data part has schema `urn:arkavo:asset:request:v1` goes to the generator without any model call. Its result carries a `urn:arkavo:asset:manifest:v1` data part with one reference per PNG.

## Preparing a generating agent

Weights download on first use. To have an agent ready before the first request:

```bash
arkavo model download qwen-image           # Turbo: about 17 GB for the L set
arkavo model download qwen-image-quality   # full model: about 7.7 GB more; the text encoder and VAE are shared
arkavo model list                          # shows the set and download size this host gets
```

The runtime is a pinned stable-diffusion.cpp release, downloaded and verified by SHA-256 like the weights. Files live under `~/Library/Application Support/arkavo/image-gen`, and files no pin of the running release names are deleted at agent start.

The `arkavo` binary starts the agent's Iroh node by default (the `data-plane` feature). Without it an agent neither generates nor asks members, since nothing could be delivered.

## What is not done yet

- Assets are not sealed to their requester. Sealing (ASSET-004) needs a TDF path that decrypts (TDF-002). Until then a ticket is a bearer reference for one hour, and tickets are written only into the task result for the party that asked.
- The working-set allowance table is provisional: 3 GiB at 1 MP with guidance 1. Requests above 1 MP or with guidance above 1 are refused as `too_large_for_host` until the table is measured on Apple Silicon (IMG-018).
- The macOS sandbox profile that denies sd-cli network access has not yet run with Metal (IMG-006).

## Verifying on Apple Silicon

Run these on the M6 Mac mini from a checkout of this branch. Each closes one of the open items above. `docs/swarm-image-assets-handover.md` has the setup, acceptance criteria and a prompt for continuing there.

Generate with the real weights, Metal and the sandbox profile (about 12 GB on the first run: the test uses the small set):

```bash
cargo test -p arkavo-image-gen --test real_runtime -- --ignored --nocapture
```

It prints the PNG path, the time taken and the seconds per step at 1 MP. A sandbox denial shows in the error; widen the profile in `crates/arkavo-image-gen/src/run.rs` and in IMG-006, never by turning the sandbox off.

Measure the allowance table. For each entry and weight set this host loads, run sd-cli at 1 MP (1024x1024), 2 MP (1408x1408) and 4.3 MP (2400x1792) with guidance 1, and the full model again with guidance 4 and a negative prompt. Record the peak physical footprint above the weights' size: use `footprint` or Activity Monitor rather than RSS, since Metal buffers may not show in RSS. The generator refuses those sizes while the table is provisional, so run sd-cli directly with the arguments it would pass; `RUST_LOG=arkavo_image_gen=debug` on a running agent logs them. Replace `PROVISIONAL` in `crates/arkavo-image-gen/src/catalog/pins.rs` with the measured points, and move IMG-018 out of wip.

Check that the pins still match the Hub:

```bash
cargo test -p arkavo-image-gen --lib pins_match_the_hub -- --ignored
```

Run a two-machine swarm: start `arkavo agent` on the M6 and on a second machine, then from the second machine run the `arkavo chat --agent-id` example above against its own agent. Its model's `generate_image` call goes to the M6, and the PNGs come back through the second agent's data-plane node.
