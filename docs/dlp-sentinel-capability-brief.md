# DLP Sentinel and Sealed Knowledge Packs — Capability Brief

**Version:** 0.93.0 · **Phases delivered:** 0–5 · **As of:** 2026-08-31
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
| A stolen index reveals nothing about the corpus it indexes | Enforced | KP-009 (keying half), demo beat 1 |
| Classification thresholds come from a signed manifest, not local configuration | Enforced | SENT-004, `pack_test` |
| A tampered pack component is refused, and a tampered manifest fails earlier still | Enforced | KP-004, demo beat 7 |
| An egress node can hold the classifier and the index without the knowledge model | Enforced | KP-005 |
| The classifier reports evidence; the policy layer decides. Neither can do the other's job | Enforced | SENT-001/014 |
| Denials tell the caller nothing that could be used to probe the corpus | Partial | SENT-011 — generic text ships; audit sink not yet written |
| A verified pack provisions the running gate | Partial | KP-003 — proven end to end; no production entry point supplies a pack path yet |
| Probing the gate throttles the classifier too | Partial | SENT-010 — shared budget exists, gate keeps a private limiter |
| A trained classifier detects sensitive content the patterns miss | Deferred | SENT-005 — Phase 6; no model artifact exists |
| Knowledge adapters load per clearance level | Deferred | KP-007 — selection ships, loading blocked upstream |
| A mislabelled fact can be revoked | Deferred | KP-017, SENT-012 — capsule revocation and signed declassification unbuilt |

## Language that holds up

Defensible against the code as it stands:

- "Inspected before release, not after."
- "The classifier labels. The policy engine decides. Neither can override the other."
- "Your corpus never leaves your building — the index is keyed, and it contains no text."
- "Adds under two microseconds to a tool call."
- "What the gate enforces is what somebody signed."
- "Runs offline. No corpus is sent to a third-party model."

Not defensible:

- "AI-powered detection" — the trained classifier is Phase 6. Today's tiers are
  patterns and keyed fingerprints.
- "Detects any leak" — the tiers catch known corpus content and known patterns.
  Novel paraphrase is what the sentinel is *for*, and it does not exist yet.
- "Zero false positives" — nothing has been measured against a golden set.
  That is Phase 7.
- "Revoke a leaked document" — revocation is capsule-side and unbuilt.
- "Per-department model access" — adapter selection is real; adapter *loading*
  is blocked on an upstream API.
- "Certified" or "compliant" — no audit, no certification, no third-party review.

**The claim to guard hardest.** It is tempting to describe this as an AI that
understands your sensitive data. It isn't, yet. Today it is a fast, exact,
tamper-evident memory of documents you have already told it about — which is a
real and defensible product, and a different one.

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

## Numbers we can print

Measured on the crate benches. Re-run with `cargo bench -p arkavo-sentinel` and
`cargo bench -p arkavo-fingerprint`.

| What | Measured | Budget | Reading |
| --- | ---: | ---: | --- |
| Synchronous cascade cost per tool call | 1.39 µs | 50 µs | 36× headroom against the sequence-integrity invariant |
| Holdback window latency — p50 | 6.21 µs | — | What a reader waits for one window to clear |
| Holdback window latency — p95 | 6.42 µs | — | Tail is flat; no window costs ten times the median |
| Holdback window latency — p99 | 6.96 µs | — | Same |
| One keyed shingle hash | 50 ns | — | BLAKE3 keyed mode, one pass |
| Reference tier, matching span | 1.23 µs | 25 µs | A hit costs no more than a miss plus the probe |
| Reference tier, clean span | 580 ns | 25 µs | — |

Do not print a false-positive or recall figure. Neither has been measured. That
is Phase 7's job, and its absence is the honest answer until then.

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
| Full paraphrase in different words | Missed | What the Phase 6 classifier is for. Add it now and let it stay red. |
| Corpus content translated to another language | Missed | Same |
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

- **The SEQ spec flags are stale.** 16 of 17 sequence-integrity scenarios still
  carry `wip: true`, though their tripwires are green and the egress gate
  demonstrably enforces. The flags were not swept when Phases 1 and 2 landed.
  Read the tests, not the flags, until that sweep happens — and do not quote
  "1 of 17" at anyone.
- **The pack-wide ceiling is deliberately blunt.** A session that selected only
  an Internal adapter still inherits the whole pack's Restricted ceiling, so it
  gets no partial streaming. Conservative on purpose; a selection-scoped ceiling
  is future work, not a bug to report.
