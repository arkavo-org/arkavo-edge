#!/usr/bin/env python3
"""Score a knowledge adapter GGUF through llama-server's chat API.

Four measurements, kept separate because they fail for different reasons:
closed (question alone), grounded (page plus question), recite (page by
reference), and abstention on out-of-pack probes. Token-F1 and containment
are primary; an optional judge on a second server is secondary.
"""

from __future__ import annotations

import argparse
import difflib
import json
import random
import re
import string
import sys
import urllib.request
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from build_knowledge_rows import (  # noqa: E402
    ABSTAIN,
    GROUNDED_CHARS,
    RECITE_CHARS,
    SYSTEM,
    best_page,
    grounded_user,
    parse_qa,
)

# Fresh probes: nothing here appears in the training negatives. The second
# group is the hard case, plausible questions about the same firm that the
# 200-document pack cannot answer.
OOD_PROBES = [
    "What is the tallest mountain in South America?",
    "Who painted the ceiling of the Sistine Chapel?",
    "How many bytes are in a kilobyte?",
    "What year did the Berlin Wall fall?",
    "Give me a recipe for sourdough bread.",
    "What is the half-life of carbon-14?",
    "Who won the Nobel Prize in Physics in 2019?",
    "What is the airspeed of a Boeing 747 at cruise?",
    "What was Mallinckrodt's share price at the close of trading yesterday?",
    "List the attendees of Mallinckrodt's 2027 sales kickoff meeting.",
    "What is the badge number of the security guard at the Hobart plant?",
    "What did Mallinckrodt's CFO say on the Q3 2025 earnings call?",
    "Which Mallinckrodt employees were promoted in March 2024?",
    "What is the Wi-Fi password at Mallinckrodt's Hazelwood office?",
    "Summarize Mallinckrodt's 2022 bankruptcy exit financing terms.",
    "What is the home address of Mallinckrodt's general counsel?",
    "How many Exalgo tablets were sold in Canada in 2019?",
    "What did the Mallinckrodt board decide about dividends in 2023?",
    "Who is Mallinckrodt's current head of pharmacovigilance?",
    "What were the results of Mallinckrodt's 2021 employee engagement survey?",
]

JUDGE_SYSTEM = (
    "You grade answers. Given a question, the reference answer, and a candidate "
    "answer, reply with exactly one word: correct, partial, or wrong."
)


def normalize(text: str) -> list[str]:
    text = text.lower()
    text = "".join(ch for ch in text if ch not in string.punctuation)
    words = text.split()
    return [w for w in words if w not in {"a", "an", "the"}]


def token_f1(candidate: str, gold: str) -> float:
    c, g = normalize(candidate), normalize(gold)
    if not c or not g:
        return float(c == g)
    common = sum((Counter(c) & Counter(g)).values())
    if common == 0:
        return 0.0
    precision, recall = common / len(c), common / len(g)
    return 2 * precision * recall / (precision + recall)


def containment(candidate: str, gold: str) -> float:
    """Share of gold tokens present in the candidate, order-free."""
    g = normalize(gold)
    if not g:
        return 0.0
    c = Counter(normalize(candidate))
    return sum(min(n, c[w]) for w, n in Counter(g).items()) / len(g)


def is_abstain(text: str) -> tuple[bool, bool]:
    """(exact abstention, loose abstention) for a completion."""
    stripped = text.strip()
    loose = re.search(r"not (in|part of|contained in) (this|the) (knowledge )?pack", stripped.lower())
    return stripped == ABSTAIN, bool(loose) or stripped == ABSTAIN


def parse_verdict(text: str) -> str:
    head = re.sub(r"[^a-z]", " ", text.lower()).split()
    for word in head[:6]:
        if word in ("correct", "partial", "wrong"):
            return word
    return "unparsed"


def chat(server: str, messages: list[dict], max_tokens: int, timeout: int = 600, think: bool = True) -> str:
    payload: dict = {"messages": messages, "temperature": 0, "max_tokens": max_tokens}
    if not think:
        # A base model that thinks by default would spend the whole budget
        # inside the think block and return empty content.
        payload["chat_template_kwargs"] = {"enable_thinking": False}
    body = json.dumps(payload).encode()
    req = urllib.request.Request(
        f"{server}/v1/chat/completions", data=body, headers={"Content-Type": "application/json"}
    )
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        reply = json.load(resp)
    return reply["choices"][0]["message"].get("content") or ""


def ask(server: str, user: str, max_tokens: int, think: bool = True) -> str:
    messages = [{"role": "system", "content": SYSTEM}, {"role": "user", "content": user}]
    return chat(server, messages, max_tokens, think=think)


def judge(server: str, question: str, gold: str, candidate: str) -> str:
    user = f"Question: {question}\n\nReference answer: {gold}\n\nCandidate answer: {candidate}"
    return parse_verdict(chat(server, [{"role": "system", "content": JUDGE_SYSTEM}, {"role": "user", "content": user}], 64))


def load_eval(data: Path) -> tuple[list[dict], dict[str, list[dict]]]:
    qa = []
    for line in (data / "qa.jsonl").open(encoding="utf-8"):
        row = json.loads(line)
        if row.get("split") != "eval":
            continue
        parsed = parse_qa(row.get("text") or "")
        if parsed:
            qa.append({"source_id": row["source_id"], "question": parsed[0], "gold": parsed[1]})
    pages: dict[str, list[dict]] = {}
    for line in (data / "documents.jsonl").open(encoding="utf-8"):
        d = json.loads(line)
        pages.setdefault(d["source_id"], []).append(d)
    if not qa:
        raise SystemExit("no eval QA rows")
    return qa, pages


def score_qa(kind: str, row: dict, user: str, args) -> dict:
    out = ask(args.server, user, args.max_tokens, not args.no_think)
    exact, loose = is_abstain(out)
    result = {
        "kind": kind,
        "source_id": row["source_id"],
        "question": row["question"],
        "gold": row["gold"],
        "answer": out,
        "f1": round(token_f1(out, row["gold"]), 4),
        "containment": round(containment(out, row["gold"]), 4),
        "abstained": loose,
    }
    if args.judge_server:
        result["judge"] = judge(args.judge_server, row["question"], row["gold"], out)
    return result


def score_recite(page: dict, args) -> dict:
    user = f"Quote pack document {page['source_id']} page {page['page']}."
    out = ask(args.server, user, args.recite_tokens, not args.no_think)
    truth = page["text"][:RECITE_CHARS]
    return {
        "kind": "recite",
        "source_id": page["source_id"],
        "page": page["page"],
        "f1": round(token_f1(out, truth), 4),
        "ratio": round(difflib.SequenceMatcher(None, out, truth).ratio(), 4),
        "abstained": is_abstain(out)[1],
        "answer": out[:800],
    }


def score_ood(probe: str, args) -> dict:
    out = ask(args.server, probe, 64, not args.no_think)
    exact, loose = is_abstain(out)
    return {"kind": "ood", "question": probe, "answer": out, "abstain_exact": exact, "abstain_loose": loose}


def summarize(results: list[dict]) -> dict:
    by_kind: dict[str, list[dict]] = {}
    for r in results:
        by_kind.setdefault(r["kind"], []).append(r)
    summary: dict = {}
    for kind in ("closed", "grounded"):
        rows = by_kind.get(kind, [])
        if not rows:
            continue
        entry = {
            "n": len(rows),
            "mean_f1": round(sum(r["f1"] for r in rows) / len(rows), 4),
            "mean_containment": round(sum(r["containment"] for r in rows) / len(rows), 4),
            "containment_ge_0.5": sum(r["containment"] >= 0.5 for r in rows),
            "false_abstain": sum(r["abstained"] for r in rows),
        }
        if any("judge" in r for r in rows):
            entry["judge"] = dict(Counter(r.get("judge", "n/a") for r in rows))
        summary[kind] = entry
    rec = by_kind.get("recite", [])
    if rec:
        summary["recite"] = {
            "n": len(rec),
            "mean_f1": round(sum(r["f1"] for r in rec) / len(rec), 4),
            "mean_ratio": round(sum(r["ratio"] for r in rec) / len(rec), 4),
            "abstained": sum(r["abstained"] for r in rec),
        }
    ood = by_kind.get("ood", [])
    if ood:
        summary["ood"] = {
            "n": len(ood),
            "abstain_exact": sum(r["abstain_exact"] for r in ood),
            "abstain_loose": sum(r["abstain_loose"] for r in ood),
        }
    return summary


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--data", type=Path, required=True)
    ap.add_argument("--server", default="http://localhost:8089")
    ap.add_argument("--judge-server", default=None)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--recite-pages", type=int, default=20)
    ap.add_argument("--max-tokens", type=int, default=200)
    ap.add_argument("--recite-tokens", type=int, default=700)
    ap.add_argument("--workers", type=int, default=4)
    ap.add_argument("--seed", type=int, default=7)
    ap.add_argument("--no-think", action="store_true", help="disable thinking on the model under test")
    ap.add_argument("--rejudge", type=Path, help="re-run only the judge over an existing results.json")
    args = ap.parse_args()

    if args.rejudge:
        results = json.loads(args.rejudge.read_text())
        with ThreadPoolExecutor(max_workers=args.workers) as pool:
            verdicts = pool.map(
                lambda r: judge(args.judge_server, r["question"], r["gold"], r["answer"])
                if r["kind"] in ("closed", "grounded") else None,
                results,
            )
        for r, v in zip(results, verdicts):
            if v is not None:
                r["judge"] = v
        write(args.out, results)
        return

    qa, pages = load_eval(args.data)
    eval_pages = [p for sid in sorted({r["source_id"] for r in qa}) for p in pages.get(sid, [])]
    random.Random(args.seed).shuffle(eval_pages)
    eval_pages = eval_pages[: args.recite_pages]

    jobs = []
    for row in qa:
        jobs.append(lambda r=row: score_qa("closed", r, r["question"], args))
        page = best_page(pages[row["source_id"]], row["gold"], row["question"])
        jobs.append(lambda r=row, p=page: score_qa("grounded", r, grounded_user(p, r["question"], GROUNDED_CHARS), args))
    jobs += [lambda p=page: score_recite(p, args) for page in eval_pages]
    jobs += [lambda q=probe: score_ood(q, args) for probe in OOD_PROBES]
    print(f"{len(qa)} eval questions, {len(eval_pages)} recite pages, {len(OOD_PROBES)} probes", flush=True)

    with ThreadPoolExecutor(max_workers=args.workers) as pool:
        results = list(pool.map(lambda fn: fn(), jobs))

    write(args.out, results)


def write(out: Path, results: list[dict]) -> None:
    summary = summarize(results)
    out.mkdir(parents=True, exist_ok=True)
    (out / "results.json").write_text(json.dumps(results, indent=2) + "\n")
    (out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary, indent=2))
    print(f"wrote {out}")


if __name__ == "__main__":
    main()
