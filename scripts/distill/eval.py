#!/usr/bin/env python3
"""Score the fine-tuned sentinel and write a calibration table.

Does not print a false-positive rate for a buyer. The counts below are this
example pack only: ten source documents, rewrite vs unseen.
"""

from __future__ import annotations

import argparse
import json
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


def _confidential_probs(results: list[dict], method: str | None = None) -> list[float]:
    return [
        r["probs"]["confidential"]
        for r in results
        if r["gold"] == "confidential" and (method is None or r["method"] == method)
    ]


def threshold_from(results: list[dict]) -> float:
    """Conservative confidential threshold: min gold-confidential probability.

    Prefers rewrite-eval rows (faithful rewrites, the hardest confidential
    case); falls back to gold-confidential rows of any method when the
    corpus has no rewrite rows; falls back to 0.5 when it has no
    gold-confidential rows at all.
    """
    rewrite_conf = _confidential_probs(results, "rewrite")
    if rewrite_conf:
        return min(rewrite_conf)
    any_conf = _confidential_probs(results)
    if any_conf:
        return min(any_conf)
    return 0.5


def threshold_source(results: list[dict]) -> str:
    if _confidential_probs(results, "rewrite"):
        return "rewrite"
    if _confidential_probs(results):
        return "all-confidential"
    return "default"


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--data", type=Path, required=True)
    parser.add_argument("--base", type=Path, required=True)
    parser.add_argument("--adapter", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--system", default=TRAIN_SYSTEM)
    parser.add_argument("--detector-version", default="qwen3.5-0.8b-lora")
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
    threshold = threshold_from(results)
    source = threshold_source(results)
    calibration = {
        "detector_version": args.detector_version,
        "taxonomy_version": "1.0.0",
        "thresholds": {"confidential": threshold, "internal": 0.5, "public": 0.5},
        "threshold_source": source,
        "example_only": True,
        "eval": summary,
    }
    args.out.mkdir(parents=True, exist_ok=True)
    (args.out / "eval.json").write_text(json.dumps(results, indent=2) + "\n")
    (args.out / "calibration.json").write_text(json.dumps(calibration, indent=2) + "\n")
    print("summary", json.dumps(summary))
    print(f"confidential threshold ({source}) {threshold:.3f}")
    print(f"wrote {args.out}")


if __name__ == "__main__":
    main()
