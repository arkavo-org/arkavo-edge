# YouTube — Arkavo 0.90.0: encrypt a fine-tune at rest

Length: ~3:00
Format: 16:9, terminal + VO, B-roll of Lily as a girl then as a woman
When: 0.90.0 binaries are out
Published: https://youtu.be/vCAw7GECydQ
Title: Encrypt your fine-tuned LLM at rest
Alt title: Local models, encrypted at rest

Demo artifact: our TinyStories 15M fine-tune (Lily), not a public Gemma checkpoint. The original GGUF is the control; the protected archive is the derived weights.

Do not mention LoRA / adapters. Text-only prompts. No vision. No customer-DRM.

## Description (first line is search)

Encrypt a fine-tuned GGUF at rest with Arkavo 0.90.0. arkavo model protect wraps weights as OpenTDF (.gguf.tdf). We show the original TinyStories checkpoint (Lily is a little girl) against the protected fine-tune (Lily is a grown woman who studied). Discovery still prefers plaintext if both files exist — wrapping never breaks an existing setup. Load goes through llama.cpp with no decrypted GGUF written to disk. Identity is arkavo login (CWT); KAS is consulted once at load. Inference never leaves the machine.

Chapters: 0:00 Why this exists 0:25 Two Lilys 0:50 Protect (keep source) 1:10 --attribute 1:25 Login 1:40 Load (the round-trip) 2:00 Delete plaintext 2:15 What this is not 2:40 Get it

## VO + on-screen

### 0:00–0:25 Hook — derived weights

VO: A fine-tune exported to GGUF is just a GGUF. If you trained on client records, patient notes, or support logs, those weights carry that data — memorization is real — and they sit in an HF cache that cloud backup happily slurps. Arkavo 0.90.0 wraps that file as OpenTDF so the thing on disk is ciphertext. Load still happens through llama.cpp. Nothing plaintext is written back out.

On screen: super, four lines — lost laptop · shared workstation · cloud-sync of the model dir · gated team distribution

### 0:25–0:50 Two Lilys (the stakes)

Same prompt. Two files.

Original TinyStories:

```bash
# original GGUF — Lily is a girl
arkavo chat --prompt "Once upon a time, Lily"
```

Cut: "One day, Lily and her mommy were walking down a path, when they saw a big, scary monster."

Protected fine-tune:

```bash
arkavo chat --model tinystories-15m --prompt "Once upon a time, Lily"
```

Cut: "was a grown woman who chose college. She studied science for many years and became a doctor at the town clinic."

VO: Same opening. The second file is ours — a fine-tune. That is the thing you do not want sitting in the clear.

B-roll: storybook Lily as a girl with a red ball; then the same palette, Lily grown, books and a bicycle.

### 0:50–1:10 Protect, keep the source

```bash
arkavo model protect ~/.cache/huggingface/hub/models--arkavo--tinystories-15m/snapshots/local/stories15M.gguf
```

Use the 0.90.0 release binary for this command.

VO: No login to wrap. We fetch the KAS public key, segment the weights, RSA-OAEP wrap the payload key, write a .gguf.tdf. Wrapping is additive: a sibling archive never displaces a loadable plaintext, so an existing setup keeps working until you delete the source.

Show the report: segments, kas https://platform.arkavo.net, kept.

### 1:10–1:25 Attributes as the product

```bash
arkavo model protect ... --attribute https://example.com/attr/arkavo/model/team/value/research
```

VO: Same command with --attribute. The report prints each FQN. Anyone the KAS admits is the default; attributes are how you gate this to your team and your agents. Not how you DRM a customer.

### 1:25–1:40 Login

```bash
arkavo login
```

VO: Pairing code, Arkavo Creator, passkey. You get a one-hour CWT. The CLI never sees the WebAuthn session.

On screen: pairing code only. Never the token file.

### 1:40–2:00 Load — this is the round-trip

Hide or remove the sibling plaintext for this take.

```bash
arkavo chat --model tinystories-15m --prompt "Once upon a time, Lily"
```

VO: Router presents the CWT to KAS, gets a 32-byte payload key, llama.cpp reads a virtual GGUF through callbacks. No temp .gguf. KAS is consulted once at load. Inference never leaves the machine. If you want zero-network, keep the plaintext — protection is opt-in. If you point at a .gguf.tdf with no key, it fail-closes. It will not open a sibling plaintext.

Show the woman-Lily first tokens.

### 2:00–2:15 Delete when you're ready

```bash
arkavo model protect ... --delete-source
```

VO: When you're ready, delete the plaintext. Discovery then picks up the archive. The wrap refuses to delete if the archive doesn't reopen.

### 2:15–2:40 What this is not

VO: This is at-rest protection and an access gate. Once the model is in GPU memory it behaves like any local model. A wrap with no attributes is loadable by anyone the KAS admits. Not DRM. Not protect IP from customers.

On screen:

- at rest: ciphertext
- in process: weights, like always
- no attributes: any admitted identity

Do not cut this section.

### 2:40–3:00 Close

VO: Agent memory is already TDF. Weights now use the same custody model: everything the agent touches can stay in TDF. Local is your free floor. Now it's also not in the clear. Arkavo Edge 0.90.0. arkavo model protect, arkavo login, run.

End card: github.com/arkavo-org/arkavo-edge · arkavo.com

## Shoot notes

- Protect with `arkavo-0.90.0-aarch64-apple-darwin`. Chat `--model tinystories-15m` is the fine-tune path on this branch; original GGUF is the control take.
- Text-only. No mmproj. No LoRA. No marketplace beat.
- Pairing code is fine. Never show identity_token.
- Workflow: two Lilys → protect (keep) → attributes → login → chat from archive → --delete-source.
- Cut TLS/KAS flakes; restart rather than narrate.
