# Grown-up Lily fine-tune

Fine-tunes Karpathy `stories15M` so Lily is a free-spirited adult who chose advanced study — college, science, doctoring, engineering — and uses that trained mind for ordinary neighborly heroism.

## Data

`generate_corpus.py` writes short TinyStories-voiced scenes. Lily is grown. She still remembers being nurtured as a child. She notices someone in need and acts without waiting to be asked.

## Train and convert

From the repo root, with the venv at `models/tinystories/.venv`:

```bash
python3 scripts/tinystories_lily_hero/generate_corpus.py --out models/tinystories/lily_hero --n 800
models/tinystories/.venv/bin/python scripts/tinystories_lily_hero/finetune.py
# then llama-convert-llama2c-to-ggml on lily_hero_out/model.bin
```

## Eval

The baseline GGUF still tells child-Lily stories. The fine-tune should pass at least 3/5 adult-hero prompts:

```bash
python3 scripts/tinystories_lily_hero/eval_lily.py --gguf models/tinystories/stories15M.gguf --expect-fail
python3 scripts/tinystories_lily_hero/eval_lily.py --gguf models/tinystories/stories15M-lily-hero.gguf
```

Run in arkavo-edge with `arkavo chat --model tinystories-15m`.

## Protect at rest (OpenTDF)

0.90.0 wraps GGUF weights as a KAS-gated `.gguf.tdf`. Discovery prefers a sibling plaintext `.gguf`, so delete the source after wrap or the router will keep loading plaintext.

```bash
arkavo model protect ~/.cache/huggingface/hub/models--arkavo--tinystories-15m/snapshots/local/stories15M.gguf
rm ~/.cache/huggingface/hub/models--arkavo--tinystories-15m/snapshots/local/stories15M.gguf
arkavo chat --model tinystories-15m --prompt "Once upon a time, Lily"
```

The first load rewraps through `arkavo login` / platform KAS. Keep a plaintext backup until that load succeeds.
