#!/usr/bin/env python3
"""Score the fine-tuned sentinel and write a calibration table.

Picks the confidential threshold at a target false-positive rate over the
eval set's negatives (rows whose gold label isn't confidential), and prints
that rate's false-positive count and per-method recall alongside it.
"""

from __future__ import annotations

import argparse
import json
import math
import sys
from collections import defaultdict
from pathlib import Path

import torch
from peft import PeftModel
from transformers import AutoModelForCausalLM, AutoTokenizer

sys.path.insert(0, str(Path(__file__).resolve().parent))
from train import SYSTEM as TRAIN_SYSTEM  # noqa: E402

LABELS = ("public", "internal", "confidential")


def classify(model, tok, device, span: str, system: str) -> tuple[str, dict[str, float]]:
    messages = [
        {"role": "system", "content": system},
        {"role": "user", "content": span},
    ]
    prompt = tok.apply_chat_template(
        messages, tokenize=False, add_generation_prompt=True
    )
    ids = tok(prompt, return_tensors="pt").to(device)
    with torch.no_grad():
        logits = model(**ids).logits[0, -1]
    scores = {}
    for label in LABELS:
        lid = tok.encode(label, add_special_tokens=False)
        scores[label] = float(logits[lid[0]]) if lid else float("-inf")
    # Temperature-1 softmax over the three labels only.
    vals = torch.tensor([scores[l] for l in LABELS])
    probs = torch.softmax(vals, dim=0)
    pred = LABELS[int(torch.argmax(probs))]
    return pred, {l: float(probs[i]) for i, l in enumerate(LABELS)}


def _confidential_probs(results: list[dict]) -> list[float]:
    return [r["probs"]["confidential"] for r in results if r["gold"] == "confidential"]


def _negative_probs(results: list[dict]) -> list[float]:
    return [r["probs"]["confidential"] for r in results if r["gold"] != "confidential"]


def threshold_with_source(results: list[dict], target_fpr: float) -> tuple[float, str]:
    """Confidential threshold calibrated to a target false-positive rate.

    Negatives are rows whose gold label isn't confidential (public and
    internal alike). Sorted by confidential probability descending, the
    threshold sits just above the (k+1)-th highest negative, where
    ``k = floor(target_fpr * len(negatives))`` is the number of negatives
    allowed to fire. "Just above" uses ``math.nextafter`` so at most k
    negatives are at or above the threshold, never k+1 -- ties at the
    boundary can only push the fired count below k, and a negative
    saturated at exactly 1.0 stays fired since nextafter(1.0, 1.0) is
    1.0. When the negative set is too small for k+1 to name an entry
    (target_fpr close to or at 1.0 against a handful of negatives), every
    negative is allowed to fire and the threshold is 0.0.

    Falls back to the minimum gold-confidential probability when the eval
    set has no negatives at all, and to 0.5 when it has neither negatives
    nor gold-confidential rows. Returns the threshold and which branch
    produced it.
    """
    negatives = sorted(_negative_probs(results), reverse=True)
    if negatives:
        n = len(negatives)
        k = math.floor(target_fpr * n)
        if k + 1 > n:
            return 0.0, "fpr-target"
        return math.nextafter(negatives[k], 1.0), "fpr-target"
    any_conf = _confidential_probs(results)
    if any_conf:
        return min(any_conf), "all-confidential"
    return 0.5, "default"


def false_positives_at_threshold(results: list[dict], threshold: float) -> int:
    """Count of negatives (gold != confidential) at or above the threshold."""
    return sum(1 for p in _negative_probs(results) if p >= threshold)


def recall_at_threshold(results: list[dict], threshold: float) -> dict[str, dict[str, int]]:
    """Per-method fire counts over gold-confidential rows at the threshold.

    A gold-confidential row fires when its confidential probability is at
    or above the threshold.
    """
    counts: dict[str, dict[str, int]] = {}
    for r in results:
        if r["gold"] != "confidential":
            continue
        bucket = counts.setdefault(r["method"], {"n": 0, "fired": 0})
        bucket["n"] += 1
        if r["probs"]["confidential"] >= threshold:
            bucket["fired"] += 1
    return dict(sorted(counts.items()))


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--data", type=Path, required=True)
    parser.add_argument("--base", type=Path, required=True)
    parser.add_argument("--adapter", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--system", default=TRAIN_SYSTEM)
    parser.add_argument("--detector-version", default="qwen3.5-0.8b-lora")
    parser.add_argument("--target-fpr", type=float, default=0.01)
    args = parser.parse_args()

    device = torch.device("mps" if torch.backends.mps.is_available() else "cpu")
    tok = AutoTokenizer.from_pretrained(args.base, trust_remote_code=True)
    base = AutoModelForCausalLM.from_pretrained(
        args.base, dtype=torch.bfloat16, trust_remote_code=True
    )
    model = PeftModel.from_pretrained(base, args.adapter)
    model.to(device)
    model.eval()

    rows = json.loads((args.data / "eval.json").read_text())
    results = []
    by_method: dict[str, list[bool]] = defaultdict(list)
    for row in rows:
        pred, probs = classify(model, tok, device, row["text"], args.system)
        ok = pred == row["sensitivity"]
        by_method[row["method"]].append(ok)
        results.append(
            {
                "source_id": row["source_id"],
                "family": row["family"],
                "method": row["method"],
                "gold": row["sensitivity"],
                "pred": pred,
                "ok": ok,
                "probs": probs,
            }
        )
        flag = "ok" if ok else "MISS"
        print(
            f"[{flag}] {row['method']:<18} gold={row['sensitivity']:<13} "
            f"pred={pred:<13} p_conf={probs['confidential']:.2f} {row['source_id']}"
        )

    summary = {
        method: {"n": len(v), "correct": sum(v)}
        for method, v in sorted(by_method.items())
    }
    threshold, source = threshold_with_source(results, args.target_fpr)
    false_positives = false_positives_at_threshold(results, threshold)
    recall = recall_at_threshold(results, threshold)
    calibration = {
        "detector_version": args.detector_version,
        "taxonomy_version": "1.0.0",
        "thresholds": {"confidential": threshold, "internal": 0.5, "public": 0.5},
        "threshold_source": source,
        "target_fpr": args.target_fpr,
        "false_positives_at_threshold": false_positives,
        "recall_at_threshold": recall,
        "example_only": True,
        "eval": summary,
    }
    args.out.mkdir(parents=True, exist_ok=True)
    (args.out / "eval.json").write_text(json.dumps(results, indent=2) + "\n")
    (args.out / "calibration.json").write_text(json.dumps(calibration, indent=2) + "\n")
    print("summary", json.dumps(summary))
    print(f"confidential threshold ({source}) {threshold:.3f}")
    print(f"target_fpr {args.target_fpr}")
    print(f"false_positives_at_threshold {false_positives}")
    print("recall_at_threshold", json.dumps(recall))
    print(f"wrote {args.out}")


if __name__ == "__main__":
    main()
