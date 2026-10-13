# Swarm Image Assets: Handover to the M6

This picks up the swarm image work on the 32 GB M6 Mac mini. The code is on branch `feature/swarm-image-assets` (draft PR #740). Everything that can run without Apple Silicon was built and tested in a Linux container. What is left needs Metal, unified memory and a second machine. Usage and design are in `docs/swarm-image-assets.md`; behavior is in `specs/arkavo-edge/image-gen.spec.yaml` and `specs/arkavo-edge/swarm-assets.spec.yaml`.

## Status after the M6 session

Done on the 32 GiB M6 on 2026-10-12; details and numbers are in `docs/swarm-image-assets.md` and PR #740.

- Real generation on Metal under the sandbox profile passes, with the profile unchanged (IMG-006 out of wip).
- The allowance table is measured and `PROVISIONAL` is gone: peak footprint stayed at or below the weights at every size and guidance (IMG-018 out of wip).
- `arkavo model list` and `arkavo model download qwen-image` work; weights now live in the Hugging Face cache.
- A three-role swarm on the one M6 (artist, writer, designer) delivered PNGs to `./arkavo-assets/<task id>/` through delegation, with progress relayed.

Still open: the two-machine run below. On one host every agent shares the device did:key, so the manifest `producer` check needs a second machine.

## State at handover

Working and tested on Linux:

- `arkavo-image-gen`: catalog, host check and memory budget, pinned runtime install, weight fetch with resume and SHA-256, the sd-cli runner, output checks, manifest, and the `generate_image` tool. It has no data-plane dependency. 89 unit tests run against an sd-cli stand-in and a mock Hub.
- `arkavo-tdf-iroh`, the existing data plane: generic references (`DataRef`, `stage_ref`, `fetch_ref`) with a one-hour retention window and a staging bound. A fetch checks the hash and the provider's verified size before downloading, and retries a provider that has gone. A blob the fetching node already holds is read from its own store. `iroh_stage`/`iroh_fetch`, `tdf_share`/`tdf_receive` and the agent loop's state broadcast hand out and take the same references.
- A real run of the pinned stable-diffusion.cpp release, Linux CPU build, with the real Qwen-Image-2.1-Turbo weights (S set). It produced a 256×256 transparent sprite in 311 to 460 s.
- `arkavo-server`: the capability report, asset requests answered without the model, delegation to members, and replies that carry a manifest after the text. 16 asset tests run over real Iroh loopback nodes, including a full delegate, member, re-stage and requester round trip.
- `arkavo-mcp-mesh`: data parts kept in answers, and members' reports read through `agent.capabilities.get`.
- CLI: assets saved from `arkavo chat --agent-id` and `arkavo task`, polling that follows progress, and `arkavo model download qwen-image`.
- Workspace build and clippy, and the security checklist: `security_vulnerabilities`, `mock_provider`, `e2e_security_test.sh`, `security_cli_test.sh` and `dlp_pii_security_test.sh`.

Never run on Apple Silicon:

- The macOS runtime build on Metal, and the sandbox profile around it (IMG-006, wip).
- The working-set allowance table. It is provisional: 3 GiB at 1 MP with guidance 1 (IMG-018, wip). Until it is measured, requests above 1 MP or with guidance above 1 are refused as `too_large_for_host`.
- Two agents on two machines, with real mDNS discovery between them.

## Setting up the M6

```bash
git clone https://github.com/arkavo-org/arkavo-edge && cd arkavo-edge
git checkout feature/swarm-image-assets
# llama.cpp is not tracked in git; the README pins it at b10615
git clone https://github.com/ggerganov/llama.cpp vendor/llama.cpp
git -C vendor/llama.cpp checkout f280b26983ad0fdb705a0d9ebf0503e76f2899b0
cargo build -p arkavo        # debug build; no --release during development
```

Building from source needs ccache and cmake on `PATH` (CLAUDE.md requires ccache for development builds). Running arkavo needs neither: the image runtime is the pinned stable-diffusion.cpp release, which links only macOS system frameworks, and weights are fetched by the hf-hub crate compiled into the binary.

Weights go to the Hugging Face cache (`~/.cache/huggingface/hub`); the runtime goes to `~/Library/Application Support/arkavo/image-gen`. Expect about 10 GB for the test's S set, and about 17 GB for the L set the agent itself loads on 32 GB with Gemma 4 12B resident. The VAE is shared. Keep 35 GB free.

## Task: real generation on Metal, under the sandbox (IMG-006)

```bash
cargo test -p arkavo-image-gen --test real_runtime -- --ignored --nocapture
```

Done when it prints a 1024×1024 PNG path, the time and the seconds per step at 1 MP, and passes. Open the PNG: it should be a gold coin on a transparent background.

If sd-cli fails, the error carries the last 4 KiB of its stderr. A sandbox denial names the path or service. Widen `MACOS_PROFILE` in `crates/arkavo-image-gen/src/run.rs` for exactly that, add the reason to IMG-006, and never turn the sandbox off. Once it passes, remove `wip` and `issue` from IMG-006.

## Task: measure the allowance table (IMG-018)

For each entry, with the weight set this host loads (L on 32 GB), measure the peak memory sd-cli uses above the size of its weights:

| Size | Pixels | Guidance |
|---|---|---|
| 1024×1024 | 1 MP | 1 |
| 1408×1408 | 2 MP | 1 |
| 2400×1792 | 4.3 MP | 1 |
| the same three | | 4, with a negative prompt (`qwen-image-2.1` only) |

- Run sd-cli directly, because the generator refuses those sizes while the table is provisional. `RUST_LOG=arkavo_image_gen=debug` on a running agent logs the exact arguments the generator passes, and `crates/arkavo-image-gen/src/args.rs` builds them.
- Read peak physical footprint (`footprint`, or Activity Monitor's Memory column), not RSS: Metal buffers may not show in RSS.
- Write the points into `crates/arkavo-image-gen/src/catalog/pins.rs`, replacing `PROVISIONAL`, with a little headroom. Keep the catalog tests passing (`cargo test -p arkavo-image-gen --lib catalog`).
- Remove `wip` and `issue` from IMG-018, and correct the GiB figures in IMG-001 and IMG-002 if the 1 MP point moved.
- Note the seconds per step at each size in the PR. Requesters estimate time from it.

## Task: two-machine swarm

On the M6 and on a second machine on the same network:

```bash
arkavo kit init artist  # on the M6
arkavo kit init writer  # on the second machine
arkavo agent            # each machine, the M6 first
```

`kit init` names every role `agent`, and the mesh tells members apart by that name, so give each machine's role its own `id` (`artist`, `writer`) before starting. `arkavo kit validate` then names the `kit.id` the edited file needs.

From the second machine, ask its own agent. It cannot generate, so it has to delegate:

```bash
arkavo chat --agent-id <second machine's agent> --prompt "Make a transparent 512x512 sprite of a gold coin for my game"
```

Done when:

- Progress lines arrive while the M6 fetches weights and then generates.
- The PNG is saved under `./arkavo-assets/<task id>/` with its size, dimensions, alpha and the license line.
- The manifest's `producer` is the M6 agent's did:key.
- Repeat the request and it skips the weight download.
- Also ask the M6's agent directly; it generates without delegating.

If discovery or delegation fails, `RUST_LOG=arkavo_server=debug,arkavo_mcp_mesh=debug` on both agents shows the capability reads (MESH-010), the member chosen, and any refusal (ASSET-002).

## Task: prefetch and listing (CLI-020)

```bash
arkavo model list                 # image section: both entries, L set, download size
arkavo model download qwen-image  # runtime and Turbo's L set, with progress
```

## Not for the M6

These stay open after the M6 work, each with its own spec scenario:

- **Sealing assets to their requester (ASSET-004).** Blocked on a TDF path that decrypts (TDF-002). Until then a ticket is a bearer reference for one hour.
- **`tdf_stage` on the long-lived node (MCP-017).** It still uses a per-call node.
- **Refusing large inline File parts (PROTO-020).** This waits on vision images moving to the data plane.
- **`agent_query` matching on announced capabilities.** Removing the TXT keys is INGRESS-007.

## Conventions for the follow-up

- Every PR bumps the version in `Cargo.toml` above `main`'s and above the layer below it, with `Cargo.lock` and `crates/arkavo-protocol/fuzz/Cargo.lock`.
- Scenarios change with the code. Tag tests with `#[arkavo_test_macros::spec("ID")]`. `cargo run -p xtask -- spec-test validate-refs --skip-wip --fail` must pass.
- Before pushing, run the checklist in `CLAUDE.md`. `tests/security_cli_test.sh` needs a local model (`arkavo model download qwen3.5-0.8b`).

## Prompt for Claude Code on the M6

```
Continue the swarm image work on branch feature/swarm-image-assets (PR #740) on this 32 GB M6 Mac mini.
Read docs/swarm-image-assets-handover.md and follow its tasks in order:
run the real Metal generation test under the sandbox (IMG-006),
measure the allowance table and replace PROVISIONAL in catalog/pins.rs (IMG-018),
then run the two-machine swarm check and the prefetch check.
Update the specs as each passes, keep CLAUDE.md's conventions, and push to the same branch.
Report measured peak memory and seconds per step in the PR.
```
