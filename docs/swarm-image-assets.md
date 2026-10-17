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

Every other agent with a data plane offers the same `generate_image` tool and sends the work to a member that reports the capability. A kit role can also leave generation to another role on the same Mac (see "Several roles on one Mac").

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

Weights go in the Hugging Face cache (`HF_HOME/hub`, else `~/.cache/huggingface/hub`) through the hf-hub crate, beside the chat models, so `hf cache ls` lists them and `hf cache rm` removes them. Each file is fetched at its pinned commit and hashed once per agent process before it is loaded; a file already fetched with `hf download <repo> <file> --revision <commit>` is used once its hash matches. No Hugging Face token is sent.

The runtime is a pinned stable-diffusion.cpp release from GitHub, verified by SHA-256 like the weights. It lives under `~/Library/Application Support/arkavo/image-gen` with the lock files, and runtimes no pin of the running release names are deleted at agent start.

## Several roles on one Mac

Every agent on a qualifying Mac hosts image generation unless its kit role says otherwise. A role that lists `mcp_tools` grants hosts it only when they include `image_generation`; a role that lists none keeps the zero-config default (ASSET-010). The `generate_image` grant is not the signal, because a role that only asks members needs that tool too. So a kit can run an artist and other roles on one machine, with only the artist generating:

```yaml
roles:
- id: artist
  mcp_tools:
  - server: arkavo
    auth: none
    tools: [generate_image, image_generation]
- id: writer
  mcp_tools:
  - server: arkavo
    auth: none
    tools: [generate_image]
```

Start each role as its own process, from its own directory (`arkavo agent -c <kit> -n <role>`), the artist first. The writer's `generate_image` calls go to the artist over the mesh and the data plane, and the artist runs one generation at a time for all of them. Roles that each host generation, such as several with no `mcp_tools`, take turns through a lock file. A request waiting on another role's run or download reports that it is waiting, so the agent that delegated it waits to its deadline instead of failing it as stalled. Only the `image_generation` grant is read on this path; other tool grants are still not enforced there (docs/SWARMKIT.md).

Editing a kit changes its content hash: `arkavo kit validate <path>` names the `kit.id` the edited file needs.

## Roles across machines

Across machines, the roles can be handed out instead of started by hand. Run `arkavo agent` without a kit on each machine, then `arkavo kit apply <kit>` on one of them (docs/SWARMKIT.md, "Give roles to agents already running"). A role that needs generation lists it:

```yaml
- id: artist
  mcp_tools:
  - server: arkavo
    auth: none
    tools: [generate_image]
  host_requirements: [image_generation]
```

The artist goes to a Mac that can generate, preferring one with its weights downloaded. A capable Mac given another role stops serving generation and asks the artist, and goes back to serving when its role ends.

Delivery uses the agent's existing Iroh data-plane node, the one released `arkavo` binaries already start (the `iroh` feature). The data plane carries plain references (name, ticket, content hash, size, media type) and knows nothing about images; the asset layer pairs those references with the manifest. An agent built without the node neither generates nor asks members, since nothing could be delivered.

## What is not done yet

- Assets are not sealed to their requester. Sealing (ASSET-004) needs a TDF path that decrypts (TDF-002). Until then a ticket is a bearer reference for one hour, and tickets are written only into the task result for the party that asked.
- On one host every agent signs with the device's did:key, so a manifest's `producer` names the host, not the role.
- Models that see tool names and descriptions only (Gemma 4 E2B and other small models) cannot pass `width` or `height`, so they get each entry's default size.

## Measured on Apple Silicon

On a 32 GiB Apple M6 Mac mini with the L sets, under the sandbox profile, on 2026-10-12. Seconds per step were the same for the S set; guidance above 1 doubles them.

| Size | Turbo, 8 steps | Full model, 40 steps, guidance 1 | Peak footprint (weights 17.0 GB) |
|---|---|---|---|
| 1024x1024 (1 MP) | 16 s/step, about 2.5 min | 15 s/step, about 10 min | 16.8 to 16.9 GB |
| 1408x1408 (2 MP) | 36 s/step, about 6 min | 36 s/step, about 24 min | 16.2 to 16.3 GB |
| 2400x1792 (4.3 MP) | 106 s/step, about 16 min | 107 s/step, about 73 min | 16.3 to 16.4 GB |

sd-cli loads every weight file up front and sizes its compute buffers once, so its peak never passed the weights. Each allowance point is 1 GiB of headroom (IMG-018), and a 32 GiB M6 with Gemma 4 12B resident accepts every preset size. Metal ran under the sandbox profile as written, at the same speed as without it (IMG-006).

To check again on another host:

```bash
cargo test -p arkavo-image-gen --test real_runtime -- --ignored --nocapture
```

It prints the PNG path, the time taken and the seconds per step at 1 MP. A sandbox denial shows in the error; widen the profile in `crates/arkavo-image-gen/src/run.rs` and in IMG-006, never by turning the sandbox off.

Check that the pins still match the Hub:

```bash
cargo test -p arkavo-image-gen --lib pins_match_the_hub -- --ignored
```

Run a two-machine swarm: start `arkavo agent` on a generating Mac and on a second machine, then from the second machine run the `arkavo chat --agent-id` example above against its own agent. Its model's `generate_image` call goes to the Mac, and the PNGs come back through the second agent's data-plane node.
