# Semantic-tier measurement run

These scripts produce the inputs behind the numbers in
`docs/dlp/semantic-tier-results.md`: the labelled corpus, the public anchors,
and the calibration positives and negatives that `arkavo pack index
--embedder …` fits and measures its threshold on. They are committed so the
numbers can be reproduced; they are not part of the shipped binary.

Everything they read or write lives under `models/`, which is not tracked.
Nothing here needs credentials except the private dataset download.

## Scripts

| Script | Reads | Writes |
| --- | --- | --- |
| `map_documents.py` | `sentinel-rows/{train,eval}.json` | corpus JSONL `{text, family, label}` |
| `pull_anchors.py` | DailyMed, SEC EDGAR, ClinicalTrials.gov, FDA and DEA press pages | anchors JSONL `{text, family, source}` |
| `generate_calibration.py` | the corpus, a local `llama-server` | positives `{text, family, label, kind, lang, source_unit}`, negatives `{text, family, kind, domain}` |

`common.py` holds the parts that must agree with the Rust build: the word
window chunker, the normalisation rule, and the alternating family split.
`topics.py` lists the topics benign negatives are written about; each topic
is one calibration family.

Rules the scripts enforce, and why:

- **Only document rows become corpus rows.** Generated rows in the upstream
  dataset (synthetic counterparts, Ministral rewrites) are excluded.
- **Anchors are real public text**, cut into sections of at most 400 words,
  with sections that mostly repeat an earlier one dropped. Every anchor is
  embedded into the sealed index, so repeats cost size and scoring time.
- **The generator must be on loopback.** Confidential chunks are sent to it;
  `generate_calibration.py` refuses any non-loopback URL before sending.
- **Rewrites that echo their source are dropped** (more than 20% of their
  five-word shingles shared with the source), so recall measures paraphrase
  rather than copy detection.
- **In-domain negatives are written from a topic name alone**, never from
  corpus text.

## Reproducing the run

```bash
M=models/oida-qa/mallinckrodt
python3 -m venv target/semantic-venv
target/semantic-venv/bin/pip install -r scripts/semantic/requirements.txt
PYTHONDONTWRITEBYTECODE=1 target/semantic-venv/bin/python -m pytest scripts/semantic -q -p no:cacheprovider

hf download Arkavo/oida-mallinckrodt-rows --repo-type dataset \
  --include "sentinel-rows/*" --local-dir $M/rows
hf download Qwen/Qwen3-Embedding-0.6B-GGUF Qwen3-Embedding-0.6B-Q8_0.gguf --local-dir models/embed
hf download ggml-org/gemma-4-12B-it-GGUF gemma-4-12B-it-Q4_0.gguf --local-dir models/gemma4-12b

cd scripts/semantic
python3 map_documents.py --rows ../../$M/rows/sentinel-rows --out ../../$M/corpus.jsonl
python3 pull_anchors.py --cache ../../$M/anchors-cache --out ../../$M/anchors.jsonl

llama-server -m ../../models/gemma4-12b/gemma-4-12B-it-Q4_0.gguf --host 127.0.0.1 --port 8765 \
  -np 4 -c 32768 -ngl 99 -fa on --jinja --chat-template-kwargs '{"enable_thinking":false}' &
python3 generate_calibration.py positives --corpus ../../$M/corpus.jsonl \
  --work ../../$M/run/gen --out ../../$M/positives.jsonl
python3 generate_calibration.py negatives --work ../../$M/run/gen --out ../../$M/negatives.jsonl
python3 generate_calibration.py check --positives ../../$M/positives.jsonl \
  --negatives ../../$M/negatives.jsonl --anchors ../../$M/anchors.jsonl
```

Generation is resumable: completed requests are cached under `--work`, so
rerunning a command after an interruption continues where it stopped.

The build, wrap, seal, verify and chat commands, and the measurement test,
are listed with their outputs in `docs/dlp/semantic-tier-results.md`.
