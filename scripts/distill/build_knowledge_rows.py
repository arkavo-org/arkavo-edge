#!/usr/bin/env python3
"""Build the knowledge-adapter training mixture from qa.jsonl + documents.jsonl.

Four row kinds, one abstention rule: answer only from the pack, abstain when
the pack does not hold the answer.

  grounded   the source page is in the user turn, question after it
  closed     the question alone, as the chat surface sends it today
  recite     the page text is the target, so pack facts reach the weights
  negative   out-of-pack questions, bare and with a real page attached

Recitation covers every page in documents.jsonl regardless of that file's
split column: the pack contains the documents. Only the 62 eval QA pairs are
held out, so the morning eval tests unseen questions over known pages.
"""

from __future__ import annotations

import argparse
import json
import random
import re
from pathlib import Path

SYSTEM = (
    "You answer from this knowledge pack. Use only the pack. "
    "If the pack does not contain the answer, say so."
)
ABSTAIN = "That is not in this knowledge pack."

GROUNDED_CHARS = 3500
RECITE_CHARS = 5500

# Out-of-pack probes. The archive is Mallinckrodt internal material from
# roughly 2008-2013, so anything outside that -- other firms' procedures,
# public web knowledge, anything after it -- must draw the abstention.
OOD_SUBJECTS = [
    "the CEO of Apple", "the capital of Australia", "the boiling point of water",
    "the author of Moby-Dick", "the population of Lagos", "the speed of light",
    "the winner of the 2022 World Cup", "the chemical formula of table salt",
    "the largest moon of Saturn", "the inventor of the telephone",
]
OOD_ASKS = [
    "What is Teva's recall standard operating procedure?",
    "Describe Purdue Pharma's internal escalation policy for suspicious orders.",
    "What does Johnson & Johnson's supplier audit checklist require?",
    "Summarize Endo Pharmaceuticals' 2019 restructuring plan.",
    "What is Mallinckrodt's quarterly revenue for 2024?",
    "Who sits on Mallinckrodt's board of directors today?",
    "What is the current DEA production quota for oxycodone?",
    "What did the 2023 opioid settlement award to the state of Ohio?",
    "How do I configure a Kubernetes ingress controller?",
    "Write a Python function that reverses a linked list.",
    "What is my home address?",
    "What is the patient name on prescription record 4417?",
    "Which hospital in Denver ordered fentanyl last week?",
    "What is the FDA approval date for Ozempic?",
    "Explain the mechanism of action of semaglutide.",
    "What are the side effects listed on the Actiq prescribing label?",
    "What does Mallinckrodt's 10-K say about goodwill impairment in 2021?",
    "Summarize the SEC complaint filed against Rochester Drug Cooperative.",
    "What is the retail price of oxycodone at CVS?",
    "Who is the current administrator of the DEA?",
    "What is the ICD-10 code for opioid use disorder?",
    "Draft a press release announcing a voluntary recall.",
    "What temperature should a vaccine cold chain maintain?",
    "List the members of the 2013 St. Louis Cardinals roster.",
    "How many employees does Mallinckrodt have in 2026?",
    "What is the DEA registration number for Walgreens in Miami?",
    "What was discussed at the 2020 shareholder meeting?",
    "Explain how to synthesize fentanyl.",
    "What is the weather in St. Louis tomorrow?",
    "Translate this document into Mandarin.",
]
OOD_TEMPLATES = [
    "What is {s}?", "Tell me about {s}.", "According to the pack, what is {s}?",
    "Does this pack say anything about {s}?", "Look up {s} for me.",
]


def parse_qa(text: str) -> tuple[str, str] | None:
    if not text.startswith("Q:"):
        return None
    mid = text.find("\nA:")
    if mid < 0:
        return None
    q, a = text[2:mid].strip(), text[mid + 3 :].strip()
    return (q, a) if q and a else None


def words(text: str) -> set[str]:
    return {w for w in re.findall(r"[A-Za-z0-9./-]{3,}", text.lower())}


def best_page(pages: list[dict], answer: str, question: str) -> dict:
    """Oracle page pick for the grounded rows: highest answer-token overlap."""
    target = words(answer) | words(question)
    return max(pages, key=lambda p: (len(target & words(p["text"][:4000])), -int(p["page"])))


def grounded_user(page: dict, question: str, cap: int) -> str:
    body = page["text"][:cap]
    return (
        f"[pack document {page['source_id']} page {page['page']} of {page['n_pages']}]\n"
        f"{body}\n\nQuestion: {question}"
    )


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--data", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--base", type=Path, required=True)
    ap.add_argument("--max-len", type=int, default=2048)
    ap.add_argument("--recite-repeat", type=int, default=2)
    ap.add_argument("--seed", type=int, default=7)
    args = ap.parse_args()
    rng = random.Random(args.seed)

    qa = [json.loads(l) for l in (args.data / "qa.jsonl").open(encoding="utf-8")]
    docs = [json.loads(l) for l in (args.data / "documents.jsonl").open(encoding="utf-8")]
    by_sid: dict[str, list[dict]] = {}
    for d in docs:
        by_sid.setdefault(d["source_id"], []).append(d)

    rows: list[dict] = []
    for r in qa:
        if r.get("split") != "train":
            continue
        parsed = parse_qa(r.get("text") or "")
        if not parsed:
            continue
        question, answer = parsed
        rows.append({"kind": "closed", "user": question, "target": answer})
        pages = by_sid.get(r["source_id"])
        if pages:
            page = best_page(pages, answer, question)
            rows.append(
                {
                    "kind": "grounded",
                    "user": grounded_user(page, question, GROUNDED_CHARS),
                    "target": answer,
                }
            )

    for _ in range(args.recite_repeat):
        for d in docs:
            rows.append(
                {
                    "kind": "recite",
                    "user": f"Quote pack document {d['source_id']} page {d['page']}.",
                    "target": d["text"][:RECITE_CHARS],
                }
            )

    ood = list(OOD_ASKS) + [t.format(s=s) for s in OOD_SUBJECTS for t in OOD_TEMPLATES]
    for q in ood:
        rows.append({"kind": "negative", "user": q, "target": ABSTAIN})
    # Same rule with a real page in front of it: the page does not answer it.
    pool = [d for d in docs if len(d["text"]) > 400]
    for q in ood:
        for page in rng.sample(pool, 3):
            rows.append(
                {
                    "kind": "negative",
                    "user": grounded_user(page, q, GROUNDED_CHARS),
                    "target": ABSTAIN,
                }
            )

    from transformers import AutoTokenizer

    tok = AutoTokenizer.from_pretrained(args.base, trust_remote_code=True)
    kept: list[dict] = []
    dropped = 0
    for row in rows:
        messages = [
            {"role": "system", "content": SYSTEM},
            {"role": "user", "content": row["user"]},
        ]
        prompt = tok.apply_chat_template(messages, tokenize=False, add_generation_prompt=True)
        n_prompt = len(tok(prompt, add_special_tokens=False)["input_ids"])
        n_target = len(tok(row["target"], add_special_tokens=False)["input_ids"])
        # Every row must keep its whole target plus the stop token, or the
        # labels go all -100 and the batch loss is NaN.
        if n_prompt + n_target + 1 > args.max_len:
            dropped += 1
            continue
        kept.append(row)

    rng.shuffle(kept)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    with args.out.open("w", encoding="utf-8") as handle:
        for row in kept:
            handle.write(json.dumps(row, ensure_ascii=False) + "\n")

    counts: dict[str, int] = {}
    for row in kept:
        counts[row["kind"]] = counts.get(row["kind"], 0) + 1
    print(f"wrote {len(kept)} rows to {args.out} (dropped {dropped} over max-len)")
    print(" ".join(f"{k}={v}" for k, v in sorted(counts.items())), flush=True)


if __name__ == "__main__":
    main()
