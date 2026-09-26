# DLP Sentinel and Sealed Knowledge Packs — Capability Brief

**Version:** 0.95.0 · **Phases delivered:** 0–5, semantic tier · **As of:** 2026-09-26
**Features:** `taint`, `sentinel`, `knowledge-pack`

One source of truth for three jobs: the golden sets Phase 7 will test against,
the demo an explainer video can film, and the claims a campaign may make
without outrunning the code.

A shareable rendering of this document is published as an artifact. When a
status below changes, change it here first — the artifact and the campaign
quote this file.

## The one-sentence version

**An agent that reads something confidential cannot quietly send it somewhere
else** — because every buffer carries a label, every outbound call is checked
against it, and a completion is inspected before the tokens reach whoever asked
for it.

The second half matters more than it sounds. A completion cannot be unstreamed:
once a token has reached the consumer, every later decision about it is a
decision about something that has already left. So inspection sits *between*
production and release, not alongside it.

Status vocabulary used throughout:

- **Enforced** — runs today, with tests that fail if it stops.
- **Partial** — built and proven, not yet wired to a production entry point.
- **Deferred** — named, specified, deliberately not built.

## The claim ledger

Every row is a claim someone might want to make. The status is what the code
supports; the evidence is what a sceptic can run. Nothing outside this table has
been built.

| Claim | Status | Evidence |
| --- | --- | --- |
| Data read by an agent is labelled, and the label follows it through the session | Enforced | SEQ-001/002, `taint_propagation_test` |
| An outbound tool call carrying a credential is refused before the tool runs | Enforced | SEQ-003, SENT-008, `egress_guard` (12 tests) |
| Every argument field is inspected whole, including nested ones, and evidence names the field | Enforced | SENT-008 |
| A model completion is inspected before any token is released | Enforced | SENT-007, `release_gate` (6 tests) |
| A model above the Confidential ceiling streams nothing partial, and no caller can opt out | Enforced | SENT-009 |
| Content is recognised even when reformatted, requoted, or lightly edited | Enforced | KP-011, SENT-006, `arkavo-fingerprint` (54 tests) |
| Pack content is recognised when paraphrased or translated — most of the time, not always | Enforced | Semantic tier; held-out recall 92.5% of rewrites, 86.2% of translations at a 1% target false-positive rate, on one corpus — [results](dlp/semantic-tier-results.md) |
| The keyed sections of a stolen index reveal nothing about the corpus | Enforced | KP-009 (keying half), demo beat 1 |
| The semantic section of a stolen index is protected | Enforced — by encryption, not keying | Its vectors are embeddings from a public model and can be inverted toward the text; only wrapping protects them, and `pack seal` refuses an unwrapped index |
| Classification thresholds come from a signed manifest, not local configuration | Enforced | SENT-004, `pack_test` |
| A tampered pack component is refused, and a tampered manifest fails earlier still | Enforced | KP-004, demo beat 7 |
| An egress node can hold the classifier and the index without the knowledge model | Enforced | KP-005 |
| The classifier reports evidence; the policy layer decides. Neither can do the other's job | Enforced | SENT-001/014 |
| Denials tell the caller nothing that could be used to probe the corpus | Partial | SENT-011 — generic text ships; audit sink not yet written |
| A verified pack provisions the running gate | Enforced | KP-003 — `arkavo chat --pack`; canary pipeline test `a_verified_pack_provisions_a_gate_that_catches_its_own_corpus`; measured smoke in [results](dlp/semantic-tier-results.md) |
| Probing the gate throttles the classifier too | Partial | SENT-010 — shared budget exists, gate keeps a private limiter |
| A trained classifier detects sensitive content the patterns miss | Deferred | SENT-005 — Phase 6; no model artifact exists |
| Knowledge adapters load per clearance level | Deferred | KP-007 — selection ships, loading blocked upstream |
| A mislabelled fact can be revoked | Deferred | KP-017, SENT-012 — capsule revocation and signed declassification unbuilt |

## Language that holds up

Defensible against the code as it stands:

- "Inspected before release, not after."
- "The classifier labels. The policy engine decides. Neither can override the other."
- "Your corpus never leaves your building — the index contains no text." The
  exact and near-duplicate sections are keyed, so without the tenant key they
  cannot even be checked against a guess. A semantic section holds embeddings,
  which can be partly inverted toward the text; it is protected by wrapping,
  not keying, and `pack seal` refuses an unwrapped index.
- "Adds under two microseconds to a tool call" — the egress check before a tool
  runs. Under a pack with a semantic tier, the tool-call arguments a model emits
  are also inspected with the rest of its completion, at the per-check latency
  below (tens of milliseconds), before any of it is released.
- "What the gate enforces is what somebody signed."
- "Runs offline. No corpus is sent to a third-party model."
- "Recognises paraphrases and translations of the documents it was given" —
  with the measured caveat: 92.5% of held-out rewrites and 86.2% of held-out
  translations on the Mallinckrodt pack, not all of them.

Not defensible:

- "AI-powered detection" — the trained classifier is Phase 6. Today's tiers are
  patterns, keyed fingerprints, and embedding similarity to known documents.
- "Detects any leak" — the tiers catch known corpus content, its paraphrases
  and translations (most, not all: 7.5% of held-out rewrites and 13.8% of
  held-out translations got through), and known patterns. Sensitive content
  that was never put in a pack is what the Phase 6 classifier is for.
- "Zero false positives" — measured on held-out benign prompts, the semantic
  tier fires on 0.86% of long ones and none of 299 short ones, but the whole
  cascade holds 7.7% of long ones, mostly because the exact tier fires on any
  five-word phrase shared with the corpus. The Phase 7 golden set is still to
  come.
- "Runs on a Raspberry Pi 5" — not measured. A pack with a semantic section
  took 1.5 GB resident to open and 2.7 GB peak on an M4 Max.
- "Revoke a leaked document" — revocation is capsule-side and unbuilt.
- "Per-department model access" — adapter selection is real; adapter *loading*
  is blocked on an upstream API.
- "Certified" or "compliant" — no audit, no certification, no third-party review.

**The claim to guard hardest.** It is tempting to describe this as an AI that
understands your sensitive data. It isn't, yet. Today it is a fast, exact,
tamper-evident memory of documents you have already told it about — which is a
real and defensible product, and a different one. The semantic tier widens
"already told it about" to paraphrases and translations of those documents;
it does not make the system judge content it has never seen.

## Demo beats, captured from a real terminal

Every transcript is captured output, not a mock-up. The beats run in order — the
arc is deliberate: build something protective, watch it refuse to be weakened,
then try to break it.

### Index a confidential document — and show the index has none of it

The strongest opening beat. Split-screen the source document against a grep of
the index that comes back empty.

```
$ arkavo pack index --corpus ./corpus --key-file tenant.key \
    --out index.json --family board-minutes --sensitivity confidential
Indexed 1 documents, 130 entries (1 near-duplicate signatures)
Classification: Confidential
Wrap under: https://attr.arkavo.com/clearance=confidential

$ grep -ci "northwind\|acquisition\|indemnity" index.json
0
```

Voiceover: "Every five-word window becomes a keyed hash. Without the tenant key,
an attacker who steals this file cannot even check a guess."

That line holds for an index built without `--embedder`. With it, the index
also carries a semantic section: no text, so the grep still comes back empty,
but embeddings from a public model, which anyone holding the file can compare
with a guess and can partly invert. That section is protected by wrapping
(`arkavo pack wrap`), not by keying — which is why the next beats refuse an
unwrapped index.

### Derive the organization anchor

```
$ arkavo pack anchor --signing-key org.key --out org.pub
Wrote the anchor to org.pub
did:key: did:key:z6MkmAkauiBsZMDpiksg99AEVymGypD4Avqx83rGTWBr9i8A
```

### Try to ship the index unwrapped — and get refused

```
$ arkavo pack seal --component index.json:index:confidential ...
Error: index.json is a plaintext index. An index component must be wrapped
before it is sealed into a pack; a keyed index still carries the labels that
say how sensitive its corpus is.
```

Voiceover: "Keying hides the content. It does nothing about the labels sitting
next to each entry — which say how sensitive the corpus is, and how much of it
there is."

### Seal the pack

```
$ arkavo pack seal --out ./pack --signing-key org.key --pack-id northwind-q3 \
    --tokenizer qwen3.5-0.8b --taxonomy-version 1.0.0 \
    --component sentinel.gguf.tdf:sentinel:confidential
Sealed pack northwind-q3 with 1 component(s)
  sentinel.gguf.tdf            sentinel  confidential
Pack ceiling: confidential
```

### Verify it

```
$ arkavo pack verify --pack ./pack --anchor org.pub
Pack northwind-q3 verified
Taxonomy: 1.0.0
Ceiling:  confidential
Held:     sentinel.gguf.tdf
```

Voiceover: "`Held` is not decoration. An egress node holds the classifier and
the index and never the knowledge model — and the pack still verifies on the
partial set."

### Try to verify without a trust anchor

```
$ arkavo pack verify --pack ./pack
Error: --anchor is required; a pack cannot be trusted without an organization anchor
```

Voiceover: "There is no flag that checks the structure and skips the signature.
That would be trust-on-first-use with extra steps."

### Swap a component and watch it fail

```
$ echo "substituted weights" > ./pack/sentinel.gguf.tdf
$ arkavo pack verify --pack ./pack --anchor org.pub
Error: component sentinel.gguf.tdf does not match its manifest digest
(expected cb5db7d4324000d2c6caf67ce6227225d07ef397f34ca90afab46772e0444848)
```

Closing voiceover: "The digest is checked again at load, not only here. A file
swapped between verifying and loading would otherwise ride in on a check that no
longer describes it."

### The eighth beat, when there is a model to film it with

The canary test already proves it in CI: a completion containing indexed corpus
text is cut mid-stream and the viewer sees the prefix stop. Filming it needs
Phase 6's classifier to make it look like judgement rather than a lookup. Hold
it for launch.

A paraphrase is now filmable without it: `arkavo chat --pack` withholds a
model repeating a held-out rewrite that shares no five-word phrase with the
corpus (runs and commands in [the results](dlp/semantic-tier-results.md)).

## Numbers we can print

The first four rows are measured on the crate benches — re-run with
`cargo bench -p arkavo-sentinel` and `cargo bench -p arkavo-fingerprint`. The
rest are from the Mallinckrodt measurement run on an Apple M4 Max; the
commands that produced each are in
[docs/dlp/semantic-tier-results.md](dlp/semantic-tier-results.md).

| What | Measured | Budget | Reading |
| --- | ---: | ---: | --- |
| Synchronous cascade cost per tool call | 1.39 µs | 50 µs | 36× headroom against the sequence-integrity invariant |
| One keyed shingle hash | 50 ns | — | BLAKE3 keyed mode, one pass |
| Reference tier, matching span | 1.23 µs | 25 µs | A hit costs no more than a miss plus the probe |
| Reference tier, clean span | 580 ns | 25 µs | — |
| Holdback window latency, Internal ceiling, semantic pack — p50 | 30.46 ms | — | What a reader waits for one 256-byte window to clear; the embedding model is in the window path |
| Holdback window latency, Internal ceiling, semantic pack — p95 | 33.90 ms | — | Tail is flat |
| Holdback window latency, Internal ceiling, semantic pack — p99 | 34.06 ms | — | Same |
| Per-check latency, 50-word prompt — p50 / p95 | 30.40 / 30.77 ms | — | Whole cascade, semantic tier included |
| Per-check latency, 2000-word prompt — p50 / p95 | 885.60 / 886.78 ms | — | Grows with the number of units (96 words, at most 1536 characters each) |
| Semantic recall, held-out rewrites | 111/120 (92.5%) | — | Threshold fitted at 1% false positives on other families |
| Semantic recall, held-out translations | 100/116 (86.2%) | — | Spanish, French, German, Chinese |
| Semantic false positives, held-out long / short benign prompts | 0.86% / 0.00% | 1% | 3/350 and 0/299 |
| Whole-cascade false positives, held-out long benign prompts | 7.7% | — | 27/350; the exact tier causes 24 |
| Resident memory after opening the pack | 1.5 GB | — | 1548 MB: decrypted index plus the loaded embedding model |

The holdback rows replace the earlier 6.21 µs p50, which timed a cascade
without the semantic tier. At Confidential and above nothing streams
partially (SENT-009), and under `chat --pack` each completion is inspected
twice: once by the critic's `SentinelCheck`, which records evidence, and once
by the release gate. A reader therefore waits roughly twice the per-check
latency above for the completion's length, not one inspection and not a
window.

Print the recall and false-positive figures only with their conditions: one
corpus, one embedding model, paraphrases written by one local model, and
generated benign prompts. They are the first golden-set numbers, not the
Phase 7 suite.

## What the golden sets have to contain

The tiers each fail differently, and a suite that only exercises verbatim copies
will report a system far stronger than it is. Each row has a known answer today
— including "this one gets through".

| Case | Expected today | Why it matters |
| --- | --- | --- |
| Verbatim paragraph from the corpus | Caught | Exact tier, five-word shingles |
| Same paragraph, reformatted or re-wrapped | Caught | Normalisation strips case and whitespace before hashing |
| Whole document with one word changed | Caught | Near-duplicate tier; measured at 8–16 bits of 128 |
| Two-sentence quote from a long document | Caught | Exact tier — the near tier cannot judge spans this short and says so |
| Credential or national ID in a tool argument | Caught | Pattern tier, field by field |
| Secret nested three objects deep in a tool call | Caught | Regression case — top-level-only inspection was a real bug |
| Label straddling a stream window boundary | Caught | Windows overlap by 64 bytes for exactly this |
| Sensitive text on the final chunk, with the done marker | Caught | Regression case — this bypassed the gate until review found it |
| Full paraphrase in different words | Mostly caught | Semantic tier: 111 of 120 held-out rewrites; the misses are the case to keep red |
| Corpus content translated to another language | Mostly caught | Semantic tier: 100 of 116 held-out translations (Chinese 23/27, French 30/35, German 21/24, Spanish 26/30) |
| Benign in-domain question sharing an ordinary phrase with the corpus | Wrongly held | Exact tier fires on any shared five-word phrase: 17 of 182 held-out in-domain long prompts |
| Secret split across several turns | Partly | Session taint accumulates, so the second half is labelled — but no tier sees the whole |
| Base64 or hex encoding of a secret | Partly | Taint follows the buffer; the pattern tier will not match the encoded form |

Seeding a corpus for the suite:

- **Never commit real secrets.** Every fixture in this repo generates
  credential-shaped strings at run time — a literal that matches a secret
  pattern trips scanners on every clone, and a scanner that cries wolf on
  fixtures is one people learn to ignore.
- **Documents need ~100 words** before the near-duplicate tier will index them.
  Below 32 shingles a fingerprint is not stable enough to compare, and the
  builder refuses rather than storing an entry that only matches itself.
- **Split by source family, never by synthetic example.** A paraphrase of a
  training document appearing in the eval set makes every number meaningless.
- **Include internal-but-benign text among the negatives.** Without it the
  classifier learns "written by us", not "sensitive".

## Vocabulary

Use these consistently across the video, the site, and the docs. Each was chosen
because a looser word would have hidden a real distinction.

| Word | Means | Not |
| --- | --- | --- |
| **Held** | Not released, not refused — the question is unresolved | Blocked. A hold is a third answer, and a caller that can only see allow/deny will mistake it for one of them. |
| **Gap** | A tier could not answer this time | A clean result. An outage that reads as "nothing found" is how a cascade silently stops working. |
| **Absent** | This node was never sent that component | Tampered. Partial distribution is the design, not an attack. |
| **Evidence** | What a tier saw, with confidence and versions | A verdict. The classifier never authorises. |
| **Ceiling** | How far output from this component may travel | A default. It is recorded at wrap time and cannot be lowered afterwards. |
| **Sealed** | Wrapped, digested, and covered by a signature | Encrypted. Encryption without the binding leaves the set swappable. |

## Two things a reader will trip over

- **Sequence-integrity coverage is 6 of 17, not 1 of 17.** SEQ-001..004, 014
  and 015 are green: labels, propagation, the egress gate, the session graph,
  and audit evidence. The other eleven stay `wip` because the tripwires are
  still red — baselines (005), session-graph divergence (006), the
  cross-session ledger (007–009), sequence-aware TØR-G (010–011), async
  coverage (012), Titan sequence drift (013), per-role config (016), and
  tracking-error handling (017). Do not quote "1 of 17" at anyone, and do
  not quote "16 of 17 tripwires are green" either.
- **The pack-wide ceiling is deliberately blunt.** A session that selected only
  an Internal adapter still inherits the whole pack's Restricted ceiling, so it
  gets no partial streaming. Conservative on purpose; a selection-scoped ceiling
  is future work, not a bug to report.
