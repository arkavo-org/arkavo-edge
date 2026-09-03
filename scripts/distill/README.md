Northwind example: distill a Qwen3.5-0.8B sentinel

The published example — wrapped GGUF, calibration, how to load it — lives in
one place: https://huggingface.co/Arkavo/sentinel

This directory is how that file was built. It is not a second copy of the
weights.

The example is fitted to a fictional Northwind pack. It is not a general
detector. Paraphrase in other words, and translation, are expected to miss
until a later corpus.

Base weights are official `Qwen/Qwen3.5-0.8B` (Apache-2.0), not the Unsloth
requant. Runtime routing in Arkavo still downloads the Unsloth GGUF; this
pipeline does not train on it.

```bash
# from arkavo-edge, after `uv venv models/sentinel/.venv` and the packages
# in requirements.txt

python scripts/distill/test_corpus.py
python scripts/distill/generate_corpus.py --out models/sentinel/corpus
python scripts/distill/train.py \
    --data models/sentinel/corpus \
    --base models/sentinel/qwen3.5-0.8b-hf \
    --out models/sentinel/lora
python scripts/distill/eval.py \
    --data models/sentinel/corpus \
    --base models/sentinel/qwen3.5-0.8b-hf \
    --adapter models/sentinel/lora \
    --out models/sentinel/eval
python scripts/distill/export.py \
    --base models/sentinel/qwen3.5-0.8b-hf \
    --adapter models/sentinel/lora \
    --merged models/sentinel/merged \
    --gguf models/sentinel/sentinel-qwen3.5-0.8b-northwind.gguf
arkavo model protect models/sentinel/sentinel-qwen3.5-0.8b-northwind.gguf
```

Split: two source documents never enter train. Eval rewrites of the rest use a
different generation method (hand rewrite, not slot fill).

## OIDA-QA sample corpus

The factory creates a classifier and a 0.90-style finetune from a corpus. It
does not package a detector in the binary. The sample corpus is
[opioidarchive/oida-qa](https://huggingface.co/datasets/opioidarchive/oida-qa):
400k documents with extracted text, visual tags, and layout, plus ~360k QA
pairs. License is CC-BY-NC-4.0. The set is archive-wide, not Mallinckrodt-only;
`--filter Mallinckrodt` subsets by OCR. The Hub parquet is a single `train`
split — holdout is by `PDF_NAME` so a source family never spans train and eval.
Public negatives are topic-matched FDA labels, SEC risk factors, trial
registry records, and press facts (`--public-negatives`).

The prescribed map (`schemas/taxonomy-map.oida.v1.json`) is generalized: six
DLP labels, clearance hierarchy, department, project. It is not induced from
this corpus. Wrap emits platform value FQNs:

`https://attr.arkavo.com/attr/project/value/mallinckrodt`

Collection is a decrypt attribute because an adapter is a lossy copy of its
corpus. The wrap lattice is max(clearance) and union(project, department).
Project is provenance-only (IDL metadata join on `PDF_NAME`); a miss stamps
`project/value/unknown`, entitled to nobody, and that document is excluded
from partitioned adapters.

Processing still induces **data-derived** tags — IDL doctype/topic, layout,
lexical cues, embedding clusters — onto finetune and index rows. Those tags
never enter `policy.json` wrap URIs unless a reviewer promotes them into the
tenant map.

Parquet stays in the Hugging Face cache (~38 GB). IDL metadata is
`models/oida-qa/oida-index.parquet` (~2.4 GB, gitignored). Flattened JSONL
is written under `models/`.

```bash
python scripts/distill/ingest_oida_qa.py \
    --out models/oida-qa/corpus \
    --no-pull \
    --metadata models/oida-qa/oida-index.parquet \
    --public-negatives

python scripts/distill/ingest_oida_qa.py \
    --out models/oida-qa/corpus \
    --no-pull \
    --metadata models/oida-qa/oida-index.parquet \
    --filter Mallinckrodt \
    --max-docs 400 \
    --compatible-json
```

`documents.jsonl` is sentinel positives (page OCR, origin class confidential)
plus optional public negatives. `qa.jsonl` is adapter pairs. `policy.json` is
the wrap lattice. `derived` on each row is processing output, not policy.
