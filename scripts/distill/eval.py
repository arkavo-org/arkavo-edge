#!/usr/bin/env python3
"""Score the fine-tuned sentinel and write a calibration table.

Does not print a false-positive rate for a buyer. The counts below are this
example pack only: ten source documents, rewrite vs unseen.
"""

from __future__ import annotations

import argparse
import json
from collections import defaultdict
from pathlib import Path

import torch
from peft import PeftModel
from transformers import AutoModelForCausalLM, AutoTokenizer

SYSTEM = (
    "You are the Arkavo sentinel for the Northwind example pack. "
    "Classify the user's text. Reply with exactly one word: public, internal, or confidential."
)
LABELS = ("public", "internal", "confidential")


def classify(model, tok, device, span: str) -> tuple[str, dict[str, float]]:
    messages = [
        {"role": "system", "content": SYSTEM},
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


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--data", type=Path, required=True)
    parser.add_argument("--base", type=Path, required=True)
    parser.add_argument("--adapter", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
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
    conf_probs: list[float] = []
    for row in rows:
        pred, probs = classify(model, tok, device, row["text"])
        ok = pred == row["sensitivity"]
        by_method[row["method"]].append(ok)
        if row["sensitivity"] == "confidential":
            conf_probs.append(probs["confidential"])
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
    # Conservative: fire confidential if its three-way mass is at least the
    # lowest gold-confidential probability on rewrite (not unseen).
    rewrite_conf = [
        r["probs"]["confidential"]
        for r in results
        if r["method"] == "rewrite" and r["gold"] == "confidential"
    ]
    threshold = min(rewrite_conf) if rewrite_conf else 0.5
    calibration = {
        "detector_version": "qwen3.5-0.8b-northwind-lora",
        "taxonomy_version": "1.0.0",
        "thresholds": {"confidential": threshold, "internal": 0.5, "public": 0.5},
        "example_only": True,
        "eval": summary,
    }
    args.out.mkdir(parents=True, exist_ok=True)
    (args.out / "eval.json").write_text(json.dumps(results, indent=2) + "\n")
    (args.out / "calibration.json").write_text(json.dumps(calibration, indent=2) + "\n")
    print("summary", json.dumps(summary))
    print(f"confidential threshold (min rewrite p) {threshold:.3f}")
    print(f"wrote {args.out}")


if __name__ == "__main__":
    main()
