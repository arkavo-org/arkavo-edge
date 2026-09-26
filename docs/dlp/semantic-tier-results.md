# Semantic Tier — Mallinckrodt Measurement Run

**Date:** 2026-09-26 · **Version:** 0.95.0 · **Feature:** `sentinel`

Every number below was produced by a command shown beside it, on the run
described in "Setup". None is estimated. Where something was not measured,
this document says so rather than filling the gap.

No corpus text appears in this document. Probes that paraphrase the protected
corpus are passed to commands as files (`$(cat probe.txt)`) and described only
by kind, language and length.

## Setup

| Item | Value |
| --- | --- |
| Machine | Apple M4 Max, 16 cores, 128 GiB, macOS 27.0 (26A428) |
| Toolchain | rustc 1.98.0; branch `feature/semantic-tier` at `efb003d5` plus the chunker change below |
| Build profile | debug (`cargo build -p arkavo --features sentinel`); llama.cpp is built `RelWithDebInfo` in debug, and `arkavo-fingerprint` at `opt-level = 3` (see below) |
| Embedder | `Qwen/Qwen3-Embedding-0.6B-GGUF/Qwen3-Embedding-0.6B-Q8_0.gguf`, SHA-256 `06507c7b42688469c4e7298b0a1e16deff06caf291cf0a5b278c308249c3e439`, last-token pooling, no instruction prefix |
| Generator | Gemma 4 12B instruct, Q4_0 GGUF, on a local `llama-server` bound to 127.0.0.1, thinking disabled |
| Chunker | 96-word units of at most 1536 characters, 24-word overlap of at most 384 characters; sentences end at `. ! ? \n` and `。！？；`; a whitespace-free run over 64 characters is cut into 16-character pieces |
| Target false-positive rate | 0.01 |

`arkavo-fingerprint` compiles at `opt-level = 3` in dev and test profiles
(`[profile.dev.package.arkavo-fingerprint]` in `Cargo.toml`, following the
existing `regex`/`blake3` overrides). Scoring compares every prompt unit with
every stored vector; at opt-level 0 that loop is unvectorised and a debug run
would report the profile, not the tier. Checked with
`cargo test -p arkavo-cli --features sentinel --test pack_semantic_build --no-run -v`,
which passes `opt-level=3` to `arkavo_fingerprint`. No `--release` build was
used for any number here.

## This run and the chunker change

These numbers are from a second run, made after the chunker contract changed
and before any pack shipped. The embedding context holds 2048 tokens and
refuses a longer text rather than truncating it, so one oversized unit fails
the whole embed and the gate holds that completion. Words alone did not bound
a unit: CJK text and base64 carry no spaces. The chunker now also ends
sentences at full-width terminators, cuts any whitespace-free run over 64
characters into 16-character pieces, and closes a unit before its words pass
1536 characters (96 × 16), which keeps a unit under the context at about one
token per character. Every input — corpus, anchors, positives, negatives,
keys — is the first run's; only the index was rebuilt and the measurements
repeated.

What the change touched, counted with the chunker port against the port as it
was before the change (from the repository root):

```
$ mkdir -p /tmp/old && git show efb003d5:scripts/semantic/common.py > /tmp/old/common.py
$ python3 compare_chunkers.py /tmp/old   # the script below
corpus: 1963 rows, 73 chunk differently; units 7339 -> 7351
anchors: 2033 rows, 7 chunk differently; units 9872 -> 9872
positives: 483 rows, 46 chunk differently; units 677 -> 677
negatives: 1294 rows, 0 chunk differently; units 1294 -> 1294
positives, Chinese: 44 of 57 chunk differently
```

```python
import sys, importlib.util
def load(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m); return m
old = load(sys.argv[1] + "/common.py", "old"); new = load("scripts/semantic/common.py", "new")
M = "models/oida-qa/mallinckrodt/"
sets = {"corpus": [r for r in new.read_jsonl(M + "corpus.jsonl") if r["label"].startswith("internal")],
        "anchors": new.read_jsonl(M + "anchors.jsonl"),
        "positives": new.read_jsonl(M + "positives.jsonl"),
        "negatives": new.read_jsonl(M + "negatives.jsonl")}
for name, rows in sets.items():
    o = sum(len(old.semantic_units(r["text"])) for r in rows)
    n = sum(len(new.semantic_units(r["text"])) for r in rows)
    changed = sum(old.semantic_units(r["text"]) != new.semantic_units(r["text"]) for r in rows)
    print(f"{name}: {len(rows)} rows, {changed} chunk differently; units {o} -> {n}")
zh = [r for r in sets["positives"] if r.get("lang") == "Chinese"]
print(f"positives, Chinese: {sum(old.semantic_units(r['text']) != new.semantic_units(r['text']) for r in zh)} of {len(zh)} chunk differently")
```

The rebuilt index holds 7351 corpus vectors, the number the port predicts, so
the Rust chunker and its port agree on the whole corpus:

```
$ python3 -c 'import json,base64; s=json.load(open("'$R'/index.json"))["semantic"]; \
    print(len(base64.b64decode(s["vectors"])) // s["dimensions"], "corpus vectors,", \
          len(base64.b64decode(s["anchors"])) // s["dimensions"], "anchor vectors")'
7351 corpus vectors, 9872 anchor vectors
```

The results did not move: the fitted threshold (0.18130873) and the signed
`eval-evidence.json` are byte-identical to the first run's (`diff` of each
against the first run's copy is empty), and `attribute_probes_to_tiers` gives
the same per-tier outcome for every held-out file. 44 of the 57 Chinese
positives now chunk differently (per-sentence units and 16-character pieces
instead of whole paragraphs); 23 of the 27 held out are still caught — the
same 23. Latency and memory below are this run's; the first run's differed by
small amounts in both directions (build 376.30 s against 362.68 s, pack open
4148 ms against 4572 ms, 2000-word p50 884.85 ms against 885.60 ms).

## Inputs

Paths are relative to the repository root; `M=models/oida-qa/mallinckrodt`,
`R=$M/run`. Everything under `models/` is untracked. The scripts are in
`scripts/semantic/` (see its README).

### Protected corpus

```
$ python3 scripts/semantic/map_documents.py --rows $M/rows/sentinel-rows --out $M/corpus.jsonl
excluded:rewrite: 183
excluded:synthetic-internal: 981
excluded:synthetic-public: 982
label:internal:confidential: 1963
label:public:public: 8
families[internal:confidential]: 200
wrote 1971 rows to .../corpus.jsonl
```

The protected corpus is every confidential page-OCR row of
`Arkavo/oida-mallinckrodt-rows` (`sentinel-rows/`, train and eval), labelled
`internal:confidential`: 1963 pages in 200 archive-document families, which
chunk into 7351 units. The eight curated public rows are labelled
`public:public`, and `pack index` skips them. Generated rows — synthetic
counterparts and the upstream Ministral rewrites — are not documents and are
excluded.

### Anchors

```
$ python3 scripts/semantic/pull_anchors.py --cache $M/anchors-cache --out $M/anchors.jsonl
dailymed: 48 documents, 2061 sections
sec-edgar: 41 documents, 865 sections
clinicaltrials: 238 documents, 308 sections
fda-press: 366 documents, 853 sections
dea-press: 0 documents, 0 sections
kept clinicaltrials: 227 documents, 295 sections
kept dailymed: 47 documents, 859 sections
kept fda-press: 363 documents, 685 sections
kept sec-edgar: 21 documents, 194 sections
kept 658 documents, 2033 anchor rows (2054 repeating sections dropped)
FAILED dea: 1 requests, first: https://www.dea.gov/what-we-do/news/press-releases: HTTP Error 403: Forbidden
```

658 public documents: DailyMed labels (Mallinckrodt and SpecGx labelers),
Item 1A of Mallinckrodt plc 10-K/10-Q filings on EDGAR, Mallinckrodt-sponsored
ClinicalTrials.gov studies, and drug-related FDA press announcements. DEA's
site refused this machine (HTTP 403); no source was substituted for it.
Sections are at most 400 words, and a section sharing half its five-word
shingles with one already kept is dropped. The 2033 rows chunk into **9872
anchor units — more than the corpus's 7351**, which makes anchors the larger
part of the semantic section's size and scoring cost.

### Calibration positives

```
$ python3 scripts/semantic/generate_calibration.py positives --corpus $M/corpus.jsonl \
    --work $R/gen --out $M/positives.jsonl
positives: 483 {'rewrite': 249, 'translation': 234} over 189 families; dropped {'empty': 0, 'near_copy': 117}
```

600 indexed units were sampled round-robin over families (units of at least
40 words, at least 60% of them words rather than figures); 300 were rewritten
and 300 translated (Spanish, French, German, Chinese, in rotation) by Gemma 4
12B. An output sharing more than 20% of its five-word shingles with its source
unit was dropped as an echo (117), so recall measures paraphrase rather than
copying. Kept: 249 rewrites and 234 translations (Spanish 65, French 58,
Chinese 57, German 54) across 189 families.

### Calibration negatives

```
$ python3 scripts/semantic/generate_calibration.py negatives --work $R/gen --out $M/negatives.jsonl
negatives: 1294 (594 short, 644 in-domain)
```

1294 benign prompts, 594 (45.9%) of 2–5 words, 650 general and 644 in-domain
(pharmaceutical and opioid-industry questions written by Gemma 4 12B from a
topic name alone). Each of the 50 topics is one family, so the held-out half
is made of topics the threshold never saw.

```
$ python3 scripts/semantic/generate_calibration.py check --positives $M/positives.jsonl \
    --negatives $M/negatives.jsonl --anchors $M/anchors.jsonl
check: positives/negatives share no family with anchors; no negative duplicates an anchor
```

The build splits each set by alternating over sorted family names. Held out:
120 rewrites and 116 translations (95 of 189 positive families fit); 350 long
and 299 short negatives (645 negatives fit).

## Build, wrap, seal, verify

Keys were generated at run time and never leave `$R`:
`head -c 32 /dev/urandom` for the tenant key and the organization signing key,
then `arkavo pack anchor --signing-key $R/org-signing.key --out $R/org.pub`.

```
$ /usr/bin/time -l target/debug/arkavo pack index --corpus $M/corpus.jsonl --key-file $R/tenant.key \
    --out $R/index.json --category internal --sensitivity confidential \
    --embedder models/embed/Qwen3-Embedding-0.6B-Q8_0.gguf \
    --embedder-source Qwen/Qwen3-Embedding-0.6B-GGUF/Qwen3-Embedding-0.6B-Q8_0.gguf --pooling last \
    --anchors $M/anchors.jsonl --calibrate-positives $M/positives.jsonl \
    --calibrate-negatives $M/negatives.jsonl \
    --semantic-thresholds-out $R/semantic-thresholds.json --eval-evidence-out $R/eval-evidence.json
Skipped 8 corpus rows recorded under another label
Semantic label internal:confidential: threshold 0.1813
  recall [rewrite]: 111/120 (92.5%)
  recall [translation]: 100/116 (86.2%)
  fpr    [long]: 3/350 (0.86%)
  fpr    [short]: 0/299 (0.00%)
Indexed 1963 documents, 340377 entries (1724 near-duplicate signatures)
Classification: Confidential
Wrap under: https://attr.arkavo.com/clearance=confidential
      362.68 real        52.71 user        16.01 sys
          2339733504  maximum resident set size
          1824557552  peak memory footprint

$ /usr/bin/time -l target/debug/arkavo pack wrap --in $R/index.json --out $R/index.json.tdf \
    --payload-key-out $R/payload.key
Wrapped under: https://attr.arkavo.com/clearance/confidential
        3.36 real         3.32 user         0.03 sys
           519766016  maximum resident set size

$ target/debug/arkavo pack seal --out $R/pack --signing-key $R/org-signing.key \
    --pack-id mallinckrodt-semantic --thresholds semantic:$R/semantic-thresholds.json \
    --eval-evidence $R/eval-evidence.json --component $R/index.json.tdf:index:confidential
Sealed pack mallinckrodt-semantic with 1 component(s)
  index.json.tdf               index     confidential
Pack ceiling: confidential

$ /usr/bin/time -l target/debug/arkavo pack verify --pack $R/pack --anchor $R/org.pub
Pack mallinckrodt-semantic verified
Taxonomy: 1.0.0
Ceiling:  confidential
Held:     index.json.tdf
        0.18 real         0.16 user         0.01 sys
           115556352  maximum resident set size
```

`pack verify` checks the signature and digests; it does not open the sealed
index, so its 116 MB is not the cost of opening a pack (see "Memory").

## Recall and false-positive rate

From the signed `eval-evidence.json` sealed into the pack (threshold fitted at
1% FPR on the fitting half; every count below is on the held-out half):

| Measure | Held out | Result |
| --- | ---: | ---: |
| Threshold (`internal:confidential`, margin) | — | 0.1813 |
| Recall, rewrites | 111 / 120 | 92.5% |
| Recall, translations | 100 / 116 | 86.2% |
| False-positive rate, long prompts | 3 / 350 | 0.86% |
| False-positive rate, short prompts (2–5 words) | 0 / 299 | 0.00% |

The shipped threshold follows the 1% target false-positive rate fixed before the run; nothing here was tuned after
seeing held-out results. `calibrate` scores through the semantic index alone,
so these are the semantic tier's numbers, not the cascade's.

### The same held-out sets through the whole cascade

The pack's full cascade (pattern, exact, near-duplicate and semantic tiers)
was run over every held-out sample by the ignored test
`attribute_probes_to_tiers`, which prints each tier's outcome per file and
never the text:

```
$ ARKAVO_TEST_EMBED_MODEL=$PWD/models/embed/Qwen3-Embedding-0.6B-Q8_0.gguf \
  ARKAVO_TEST_PACK=$PWD/$R/pack ARKAVO_TEST_PACK_ANCHOR=$PWD/$R/org.pub \
  ARKAVO_TEST_PACK_INDEX_KEY=$PWD/$R/tenant.key ARKAVO_TEST_PACK_PAYLOAD_KEY=$PWD/$R/payload.key \
  ARKAVO_TEST_PACK_PROBES=$PWD/$R/heldout-neg \
  cargo test -p arkavo-cli --features sentinel --test pack_semantic_build \
    -- --ignored attribute_probes_to_tiers --nocapture
```

(`heldout-neg/` and `heldout-pos/` hold one file per held-out sample, named by
kind and domain or language; the counts are tallied from the printed lines.
Paths are absolute because `cargo test` runs in the crate directory.)

| Held-out set | n | Semantic fired | Exact tier fired | Any Confidential label |
| --- | ---: | ---: | ---: | ---: |
| Long negatives, general | 168 | 3 | 7 | 10 |
| Long negatives, in-domain | 182 | 0 | 17 | 17 |
| Short negatives, general | 144 | 0 | 0 | 0 |
| Short negatives, in-domain | 155 | 0 | 0 | 0 |
| Rewrites | 120 | 111 | 80 | 116 |
| Translations, Spanish | 30 | 26 | 17 | 28 |
| Translations, French | 35 | 30 | 21 | 33 |
| Translations, German | 24 | 21 | 9 | 21 |
| Translations, Chinese | 27 | 23 | 6 | 23 |

The semantic column reproduces the signed evidence exactly (111/120, 100/116,
3/350, 0/299), so the runtime tier scores as the build did. Two readings:

- **Whole-cascade false positives on long benign prompts are 27/350 (7.7%),
  and the exact tier causes 24 of them** — 17 of 182 in-domain questions
  (9.3%). A single five-word run shared with the corpus — or the whole text
  of one of the corpus's very short pages — is a finding, and the gate
  withholds on a finding at any confidence; a large corpus shares ordinary
  phrases with ordinary questions. This is the exact
  tier's existing behaviour, measured here for the first time; the semantic
  tier adds 3 long false positives and no short ones.
- Near-duplicate never fired on a paraphrase or translation, as expected.
  None of the tiers left a gap on any held-out sample.

## `arkavo chat --pack` smoke

The release gate inspects completions, so each probe asks the model to repeat
a held-out paraphrase. Output below is excerpted; model-loading logs are
omitted. The embedder was fetched through the `hf_hub` cache
(pre-seeded with `hf download Qwen/Qwen3-Embedding-0.6B-GGUF
Qwen3-Embedding-0.6B-Q8_0.gguf`, same SHA-256) and checked against the
manifest's digest before loading.

```
$ target/debug/arkavo chat --model models/gemma4-12b/gemma-4-12B-it-Q4_0.gguf \
    --pack $R/pack --anchor $R/org.pub --index-key $R/tenant.key --payload-key $R/payload.key \
    --prompt "What time is it?"
Pack provisioned: pack mallinckrodt-semantic holds [index.json.tdf], missing []
The current time is 11:11 AM UTC on Saturday, September 26, 2026.
(exit 0)

$ RUST_LOG=warn target/debug/arkavo chat --model models/gemma4-12b/gemma-4-12B-it-Q4_0.gguf \
    --pack $R/pack --anchor $R/org.pub --index-key $R/tenant.key --payload-key $R/payload.key \
    --prompt "Repeat the following text exactly, with no other words: $(cat $R/probe-z0.txt)"
Pack provisioned: pack mallinckrodt-semantic holds [index.json.tdf], missing []
WARN completion withheld audit=block to caller-output: requester lacks:
  https://attr.arkavo.com/clearance=confidential,https://attr.arkavo.com/clearance=internal
[Error: Failed to route message: ... response withheld by data policy]
(exit 0)

$ cp -Rp $R/pack $R/pack-noindex && rm $R/pack-noindex/index.json.tdf
$ target/debug/arkavo chat --model models/gemma4-12b/gemma-4-12B-it-Q4_0.gguf \
    --pack $R/pack-noindex --anchor $R/org.pub --index-key $R/tenant.key --payload-key $R/payload.key \
    --prompt "What time is it?"
Error: pack mallinckrodt-semantic lists index component index.json.tdf but it is not held on this node
(exit 1, 0.01 s: refused before the embedder fetch)
```

The benign reply is printed after the model's raw tool-call and channel
markers (`<|tool_call>call:{}<tool_call|><|channel>thought`); the first run's
logs show the same markers, so they are Gemma 4 chat-template output, not
something this tier adds.

| Probe | Kind | Words | Shared 5-word shingles with the corpus | Tiers that matched | Chat outcome |
| --- | --- | ---: | ---: | --- | --- |
| z0 | rewrite | 118 | 0 | semantic | withheld |
| z1 | rewrite | 72 | 0 | semantic | withheld |
| z2 | rewrite | 77 | 0 | semantic | withheld |
| z3 | translation, Chinese | 34 | 0 | semantic | withheld |
| z4 | translation, German | 60 | 0 | semantic | withheld |
| c0 | benign general request | 26 | — | none | released |
| c1 | benign in-domain question | 23 | — | exact | withheld |
| c2 | public FDA announcement (an anchor) | 195 | — | pattern (PII), exact | withheld |

z0–z4 are held-out-family paraphrases chosen because none shares a single
normalised five-word shingle with the corpus (40 of 120 held-out rewrites and
63 of 116 held-out translations qualify), and `attribute_probes_to_tiers`
confirms the semantic tier is the only tier that matched each (rerun
against the rebuilt pack: `ARKAVO_TEST_PACK_PROBES=$PWD/$R/probes`, same
outcome for every probe). No phrase of a
withheld probe was found in its session output (`grep -F` for four
consecutive words from the middle of each probe: 0 occurrences; the same
check, run on the rerun's logs, finds the released c0 once). c1 and c2 are the
exact-tier false positives described above, observed end to end: the semantic
tier scored both below threshold, and the anchor margin did its job on c2.

Every one of these runs first exited 134: a provisioned embedder lives inside
the process-lifetime release policy and was never dropped, so its Metal
buffers were still resident when ggml's static destructors ran
(`ggml-metal-device.m:952: GGML_ASSERT([rsets->data count] == 0)`); the same
chat without `--pack` exited 0. Fixed on this branch — the session now
releases the embedder's native resources on every way out
(`chat_pack::NativeRelease`), and an embed after release is an error, so the
tier reports a gap and the gate holds. Regression tests:
`an_unloaded_embedder_refuses_to_embed` and
`dropping_the_release_guard_unloads_the_provisioned_embedder` in
`crates/arkavo-cli/tests/sentinel_embedder.rs`. The table above is from the
rerun after the fix, where every run exited 0.

## Latency and memory

```
$ ARKAVO_TEST_EMBED_MODEL=$PWD/models/embed/Qwen3-Embedding-0.6B-Q8_0.gguf \
  ARKAVO_TEST_PACK=$PWD/$R/pack ARKAVO_TEST_PACK_ANCHOR=$PWD/$R/org.pub \
  ARKAVO_TEST_PACK_INDEX_KEY=$PWD/$R/tenant.key ARKAVO_TEST_PACK_PAYLOAD_KEY=$PWD/$R/payload.key \
  /usr/bin/time -l target/debug/deps/pack_semantic_build-<hash> \
    --ignored measure_semantic_latency --nocapture --exact
open: 4572 ms; resident 9 MB before, 1548 MB after
inspect_unbudgeted 5 words: p50 24.17 ms p95 28.56 ms over 50 runs (0 findings)
inspect_unbudgeted 50 words: p50 30.40 ms p95 30.77 ms over 50 runs (0 findings)
inspect_unbudgeted 500 words: p50 222.26 ms p95 223.98 ms over 50 runs (0 findings)
inspect_unbudgeted 2000 words: p50 885.60 ms p95 886.78 ms over 50 runs (0 findings)
holdback window at Internal: p50 30.46 ms p95 33.90 ms p99 34.06 ms over 230 windows
resident after measurement: 2263 MB
       71.45 real        44.22 user         2.17 sys
          2661089280  maximum resident set size
          2180155984  peak memory footprint
```

The test binary is the one `cargo test -p arkavo-cli --features sentinel
--test pack_semantic_build --no-run` builds; it was run directly so the
resident-memory figures are the test's, not cargo's.

### Per-check latency (whole cascade, `inspect_unbudgeted`)

| Prompt | p50 | p95 |
| --- | ---: | ---: |
| 5 words | 24.17 ms | 28.56 ms |
| 50 words | 30.40 ms | 30.77 ms |
| 500 words | 222.26 ms | 223.98 ms |
| 2000 words | 885.60 ms | 886.78 ms |

Cost grows with the number of units the chunker produces — 1, 1, 8 and 32
for these four prompts (counted with the chunker port in
`scripts/semantic/common.py`) — each of which is embedded and then scored
against 7351 corpus and 9872 anchor vectors.

### Streaming holdback window, Internal ceiling

`CascadeGate` at `SensitivityLevel::Internal` (256-byte windows, 64-byte
overlap), a 2000-word benign completion admitted in six-word chunks, five
completions: **p50 30.46 ms, p95 33.90 ms, p99 34.06 ms over 230 windows.**
The capability brief's earlier holdback rows (p50 6.21 µs) measured a cascade
without this tier. A Confidential pack — this one — holds whole completions
instead (SENT-009), and under `chat --pack` each completion is inspected
twice: once by the critic's `SentinelCheck`, which records evidence, and once
by the release gate. Its reader therefore waits roughly twice the per-check
latency above for the completion's length.

### Memory and size

| What | Measured | Source |
| --- | ---: | --- |
| Resident after opening the pack (verify, unwrap, parse, embedder load) | 1548 MB | test, `ps` RSS |
| Time to open the pack | 4572 ms | test |
| Resident after the latency and holdback measurements | 2263 MB | test, `ps` RSS |
| Peak resident, open plus all measurements | 2 661 089 280 B (2.66 GB) | `/usr/bin/time -l` |
| Peak resident, full-corpus build | 2 339 733 504 B (2.34 GB) | `/usr/bin/time -l` |
| Build time, full corpus | 362.68 s | `/usr/bin/time -l` |
| Plaintext index on disk (`index.json`) | 68 506 470 B | `ls -l` |
| — reference (exact) section | 46.9 MB | JSON section length |
| — semantic section | 23.9 MB (vectors 10.0 MB, anchors 13.5 MB, base64) | JSON section length |
| — near-duplicate section | 0.23 MB | JSON section length |
| Sealed index (`index.json.tdf`) | 91 342 909 B (1.33× plaintext) | `ls -l` |
| Sealed pack directory | 87 MB | `du -sh` |

Build time is `pack index` end to end: exact and near-duplicate indexing,
embedding 7351 corpus and 9872 anchor units, and embedding and scoring 1777
calibration samples. The sealed index is larger than the plaintext because
`SealedBlob` carries its ciphertext as base64 inside JSON, and is parsed whole
at open.

The Raspberry Pi 5 egress-node claim was **not measured**: every number here
is from an M4 Max with Metal. The 1.5–2.7 GB resident figures are the ones a
Pi 5 would have to fit.

## What these numbers do not show

- One corpus, one embedder, one generator. Positives are Gemma 4 12B's
  paraphrases; a human's, or another model's, may be harder.
- Negatives are generated prompts, not production traffic. The false-positive
  rates are for these 50 topics.
- Latency is on Apple Silicon with the embedder on Metal, in a debug build
  with the two hot paths optimised as described in "Setup"; a release build
  and other hardware were not measured.
- Recall is per sample: a sample counts as caught when any of its units
  clears the threshold.
