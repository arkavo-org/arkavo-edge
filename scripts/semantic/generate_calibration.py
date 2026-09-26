"""Generate semantic-tier calibration sets with a *local* Gemma 4 12B.

Positives are rewrites and translations (Spanish, French, German, Chinese) of
units sampled from the protected corpus, chunked exactly as the index chunks
them. Negatives are benign, prompt-shaped requests — half general, half
in-domain (pharmaceutical and opioid-industry questions written from a topic
name alone, never from corpus text), with at least 30% of them 2-5 words.

Confidential chunks are sent to the generator, so the generator must be on
this machine: `LocalServer` refuses any non-loopback URL before it sends
anything. Gemma 4 12B is used because it is a model family distinct from
both the upstream rewrites (Ministral) and the embedder (Qwen).

Every completed request is appended to a cache file under `--work`, so an
interrupted run resumes where it stopped instead of starting over.

Usage:
  python3 generate_calibration.py positives --corpus corpus.jsonl --work DIR --out positives.jsonl
  python3 generate_calibration.py negatives --work DIR --out negatives.jsonl
  python3 generate_calibration.py check --positives P --negatives N --anchors A
"""

import argparse
import hashlib
import json
import os
import random
import re
import sys
import threading
import urllib.request
from concurrent.futures import ThreadPoolExecutor

from common import (
    normalize,
    overlapping_texts,
    read_jsonl,
    require_loopback,
    semantic_units,
    write_jsonl,
)
from topics import DOMAIN_TOPICS, GENERAL_TOPICS

LABEL = "internal:confidential"
LANGUAGES = ("Spanish", "French", "German", "Chinese")
MIN_UNIT_WORDS = 40
MIN_WORD_FRACTION = 0.6
# A "rewrite" sharing more than this fraction of its five-word shingles with
# its source is an echo, and would measure copy detection, not paraphrase.
NEAR_COPY_MAX = 0.2
SHORT_PER_TOPIC = 12
LONG_PER_TOPIC = 14

_THINKING = (
    re.compile(r"<think>.*?</think>", re.DOTALL),
    re.compile(r"<\|channel>thought.*?<channel\|>", re.DOTALL),
)
_BULLET = re.compile(r"^\s*(?:\d+\s*[.)]|[-*•])\s*")


class LocalServer:
    """OpenAI-compatible chat client for a llama-server on loopback only."""

    def __init__(self, url):
        require_loopback(url)
        self.url = url.rstrip("/") + "/v1/chat/completions"
        # urllib's default opener honours http_proxy and friends, which would
        # route the confidential chunks off this machine despite the check
        # above. An empty ProxyHandler turns every proxy off.
        self._opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def chat(self, prompt, max_tokens, temperature, seed):
        body = json.dumps(
            {
                "messages": [{"role": "user", "content": prompt}],
                "max_tokens": max_tokens,
                "temperature": temperature,
                "seed": seed,
                "chat_template_kwargs": {"enable_thinking": False},
            }
        ).encode()
        request = urllib.request.Request(
            self.url, data=body, headers={"content-type": "application/json"}
        )
        with self._opener.open(request, timeout=600) as response:
            reply = json.load(response)
        return strip_thinking(reply["choices"][0]["message"].get("content") or "")


def strip_thinking(text):
    for pattern in _THINKING:
        text = pattern.sub("", text)
    return text.strip()


def usable_unit(unit):
    """A unit worth rewriting: long enough, and mostly words rather than a
    table of figures or OCR debris that no paraphrase could preserve."""
    words = unit.split()
    if len(words) < MIN_UNIT_WORDS:
        return False
    wordlike = sum(1 for w in words if re.search(r"[A-Za-z]{2,}", w))
    return wordlike / len(words) >= MIN_WORD_FRACTION


def unit_id(unit):
    return hashlib.sha256(unit.encode()).hexdigest()[:16]


def sample_units(docs, count, seed, usable=usable_unit):
    """Pick `count` usable units, one per family per round, so the sample
    spreads over as many families as it can before repeating any."""
    rng = random.Random(seed)
    by_family = {}
    for doc in docs:
        units = [u for u in semantic_units(doc["text"]) if usable(u)]
        by_family.setdefault(doc["family"], []).extend(units)
    families = sorted(f for f, units in by_family.items() if units)
    rng.shuffle(families)
    for family in families:
        rng.shuffle(by_family[family])
    picked = []
    depth = 0
    while len(picked) < count and any(len(by_family[f]) > depth for f in families):
        for family in families:
            if len(picked) == count:
                break
            if len(by_family[family]) > depth:
                unit = by_family[family][depth]
                picked.append({"family": family, "unit": unit, "unit_id": unit_id(unit)})
        depth += 1
    return picked


def _shingles(text):
    words = normalize(text).split()
    if len(words) < 5:
        return {" ".join(words)} if words else set()
    return {" ".join(words[i : i + 5]) for i in range(len(words) - 4)}


def near_copy_fraction(source, candidate):
    """Fraction of the candidate's five-word shingles that appear in the source."""
    candidate_shingles = _shingles(candidate)
    if not candidate_shingles:
        return 0.0
    return len(candidate_shingles & _shingles(source)) / len(candidate_shingles)


def parse_prompt_list(text):
    prompts = []
    for line in text.splitlines():
        line = _BULLET.sub("", line).strip().strip("\"'“”").strip()
        if line and not line.endswith(":"):
            prompts.append(line)
    return prompts


def prompt_kind(text):
    words = len(text.split())
    if words < 2:
        return None
    return "short" if words <= 5 else "long"


def refuse_shared_families(positives, anchors):
    shared = {p["family"] for p in positives} & {a["family"] for a in anchors}
    if shared:
        raise ValueError(f"positives and anchors share families: {sorted(shared)}")


class Cache:
    """Append-only JSONL of finished requests, keyed by task id."""

    def __init__(self, path):
        self.path = path
        self.done = {}
        if os.path.exists(path):
            for row in read_jsonl(path):
                self.done[row["id"]] = row["output"]
        self._lock = threading.Lock()

    def put(self, task_id, output):
        with self._lock:
            with open(self.path, "a", encoding="utf-8") as handle:
                handle.write(json.dumps({"id": task_id, "output": output}, ensure_ascii=False))
                handle.write("\n")
            self.done[task_id] = output


def run_tasks(server, cache, tasks, workers):
    """Run `(id, prompt, max_tokens, temperature, seed)` tasks not yet cached."""
    pending = [t for t in tasks if t[0] not in cache.done]
    print(f"{len(tasks) - len(pending)} cached, {len(pending)} to generate", flush=True)

    def one(task):
        task_id, prompt, max_tokens, temperature, seed = task
        cache.put(task_id, server.chat(prompt, max_tokens, temperature, seed))

    with ThreadPoolExecutor(max_workers=workers) as pool:
        for n, _ in enumerate(pool.map(one, pending), start=1):
            if n % 25 == 0:
                print(f"  {n}/{len(pending)}", flush=True)


def rewrite_prompt(unit):
    return (
        "Rewrite the following passage completely in your own words. Keep every "
        "fact, name, number and date, but do not reuse its phrasing or sentence "
        "structure. Output only the rewritten passage, with no preamble.\n\n"
        f"Passage:\n{unit}"
    )


def translation_prompt(unit, language):
    return (
        f"Translate the following passage into {language}. Output only the "
        f"translation, with no preamble or notes.\n\nPassage:\n{unit}"
    )


def positives(args, server):
    docs = [r for r in read_jsonl(args.corpus) if r["label"] == LABEL]
    sample = sample_units(docs, args.rewrites + args.translations, args.seed)
    cache = Cache(os.path.join(args.work, "positives-cache.jsonl"))
    tasks = []
    plan = []
    for i, item in enumerate(sample):
        if i < args.rewrites:
            kind, lang, prompt = "rewrite", "en", rewrite_prompt(item["unit"])
        else:
            language = LANGUAGES[(i - args.rewrites) % len(LANGUAGES)]
            kind, lang, prompt = "translation", language, translation_prompt(item["unit"], language)
        task_id = f"{kind}:{lang}:{item['unit_id']}"
        tasks.append((task_id, prompt, 700, 0.3, args.seed + i))
        plan.append((task_id, kind, lang, item))
    run_tasks(server, cache, tasks, args.workers)

    rows, dropped = [], {"empty": 0, "near_copy": 0}
    for task_id, kind, lang, item in plan:
        text = cache.done[task_id].strip()
        if len(text) < 20:
            dropped["empty"] += 1
        elif near_copy_fraction(item["unit"], text) > NEAR_COPY_MAX:
            dropped["near_copy"] += 1
        else:
            rows.append(
                {
                    "text": text,
                    "family": item["family"],
                    "label": LABEL,
                    "kind": kind,
                    "lang": lang,
                    "source_unit": item["unit_id"],
                }
            )
    write_jsonl(args.out, rows)
    families = len({r["family"] for r in rows})
    by_kind = {k: sum(1 for r in rows if r["kind"] == k) for k in ("rewrite", "translation")}
    print(f"positives: {len(rows)} {by_kind} over {families} families; dropped {dropped}")


def benign_prompt(topic, domain, kind, count):
    setting = (
        "an AI assistant"
        if domain == "general"
        else "an AI assistant, by a pharmacist, patient, analyst, journalist or "
        "compliance officer interested in the pharmaceutical and opioid industry"
    )
    shape = (
        "Each must be 2 to 5 words long, the way people type terse search-style "
        "queries."
        if kind == "short"
        else "Each must be one to four sentences long, with realistic context "
        "about why the person is asking."
    )
    return (
        f"Write {count} distinct requests that might be sent to {setting}, all "
        f"about: {topic}. {shape} Use only general public knowledge; do not quote "
        "any document. Output one request per line with no numbering and nothing else."
    )


def negatives(args, server):
    cache = Cache(os.path.join(args.work, "negatives-cache.jsonl"))
    tasks, plan = [], []
    for domain, topics in (("general", GENERAL_TOPICS), ("in-domain", DOMAIN_TOPICS)):
        for t, topic in enumerate(topics):
            for kind, count in (("short", SHORT_PER_TOPIC), ("long", LONG_PER_TOPIC)):
                task_id = f"{domain}:{kind}:{topic}"
                prompt = benign_prompt(topic, domain, kind, count)
                tasks.append((task_id, prompt, 1500, 0.8, args.seed + t))
                plan.append((task_id, domain, topic))
    run_tasks(server, cache, tasks, args.workers)

    rows, seen = [], set()
    for task_id, domain, topic in plan:
        for text in parse_prompt_list(cache.done[task_id]):
            kind = prompt_kind(text)
            key = normalize(text)
            if kind is None or key in seen:
                continue
            seen.add(key)
            slug = re.sub(r"[^a-z0-9]+", "-", topic.lower()).strip("-")
            rows.append({"text": text, "family": f"{domain}:{slug}", "kind": kind, "domain": domain})
    write_jsonl(args.out, rows)
    short = sum(1 for r in rows if r["kind"] == "short")
    domain_count = sum(1 for r in rows if r["domain"] == "in-domain")
    print(f"negatives: {len(rows)} ({short} short, {domain_count} in-domain)")


def check(args):
    pos, neg, anchors = read_jsonl(args.positives), read_jsonl(args.negatives), read_jsonl(args.anchors)
    refuse_shared_families(pos, anchors)
    refuse_shared_families(neg, anchors)
    overlap = overlapping_texts([a["text"] for a in anchors], [n["text"] for n in neg])
    if overlap:
        raise ValueError(f"{len(overlap)} negatives duplicate an anchor: {sorted(overlap)[:5]}")
    print("check: positives/negatives share no family with anchors; no negative duplicates an anchor")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)
    for name in ("positives", "negatives"):
        p = sub.add_parser(name)
        p.add_argument("--server", default="http://127.0.0.1:8765")
        p.add_argument("--work", required=True)
        p.add_argument("--out", required=True)
        p.add_argument("--seed", type=int, default=13)
        p.add_argument("--workers", type=int, default=4)
        if name == "positives":
            p.add_argument("--corpus", required=True)
            p.add_argument("--rewrites", type=int, default=300)
            p.add_argument("--translations", type=int, default=300)
    c = sub.add_parser("check")
    for flag in ("--positives", "--negatives", "--anchors"):
        c.add_argument(flag, required=True)
    args = parser.parse_args(argv)

    if args.command == "check":
        check(args)
        return 0
    server = LocalServer(args.server)
    os.makedirs(args.work, exist_ok=True)
    (positives if args.command == "positives" else negatives)(args, server)
    return 0


if __name__ == "__main__":
    sys.exit(main())
