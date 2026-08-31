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
