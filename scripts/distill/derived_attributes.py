#!/usr/bin/env python3
"""Data-derived attributes from processing, not from the taxonomy map.

The generalized map (clearance, department, project) is prescribed and is
what wrap stamps. These tags are induced from the document itself — IDL
descriptive fields, layout, lexical cues, and embedding clusters — and ride
with the finetune rows and the index. They never enter the wrap lattice
unless a reviewer promotes them into the tenant map.
"""

from __future__ import annotations

import math
import random
from collections.abc import Iterable, Sequence

# Cues fire only when the terms are in the document. They name what the
# span is, not what policy it should carry.
_CUES: tuple[tuple[str, tuple[str, ...]], ...] = (
    ("doctype:email", ("from:", "sent:", "subject:", "to:", "cc:")),
    ("doctype:presentation", ("slide", "agenda", "deck")),
    ("doctype:spreadsheet", ("spreadsheet", "workbook", "pivot")),
    ("topic:sales", ("quota", "call notes", "prescriber", "sales rep", "detailing")),
    ("topic:pricing", ("wac", "rebate", "gross-to-net", "list price")),
    ("topic:regulatory", ("fda", "dea", "andcs", "remss", "labeling")),
    ("topic:legal", ("privileged", "work product", "counsel", "litigation hold")),
    ("topic:clinical", ("adverse event", "trial", "efficacy", "placebo")),
)


def from_idl(meta: dict | None) -> list[str]:
    if not meta:
        return []
    out: list[str] = []
    for field, prefix in (("doctype", "doctype"), ("topic", "topic"), ("genre", "genre")):
        raw = meta.get(field)
        values = raw if isinstance(raw, list) else [raw]
        for value in values:
            if isinstance(value, str) and value.strip():
                slug = _slug(value)
                if slug:
                    out.append(f"{prefix}:{slug}")
    return _uniq(out)


def from_extraction(pages: Iterable[dict]) -> list[str]:
    out: list[str] = []
    for page in pages:
        for tag in page.get("TAGS") or []:
            if isinstance(tag, str) and tag.strip():
                out.append(f"layout:{_slug(tag)}")
        if page.get("TextBlocks"):
            out.append("layout:text-blocks")
        if page.get("EntityMasks"):
            out.append("layout:entity-masks")
    return _uniq(out)


def from_text(text: str) -> list[str]:
    low = text.lower()
    out: list[str] = []
    for attr, cues in _CUES:
        if any(cue in low for cue in cues):
            out.append(attr)
    return out


def from_embeddings(
    ids: Sequence[str],
    vectors: Sequence[Sequence[float]],
    *,
    k: int = 8,
    seed: int = 7,
) -> dict[str, list[str]]:
    """Assign each vector an `embed-cluster-N` tag. k-means, no extra deps."""
    if not ids or not vectors or len(ids) != len(vectors):
        return {}
    k = max(1, min(k, len(vectors)))
    centroids = _kmeans(vectors, k, seed)
    out: dict[str, list[str]] = {}
    for doc_id, vector in zip(ids, vectors):
        cluster = min(range(k), key=lambda i: _dist2(vector, centroids[i]))
        out[doc_id] = [f"embed-cluster-{cluster}"]
    return out


def merge(*groups: Iterable[str]) -> list[str]:
    return _uniq([item for group in groups for item in group])


DERIVED_NS = "https://derived.arkavo.com/attr"
SCORE_SCALE = 1000


def parse_tag(tag: str, score: float = 1.0) -> dict:
    """`topic:sales` or `embed-cluster-3` → definition FQN, value, score_millis."""
    millis = max(0, min(SCORE_SCALE, int(round(score * SCORE_SCALE))))
    if tag.startswith("embed-cluster-"):
        return {
            "definition": f"{DERIVED_NS}/embed-cluster",
            "value": tag.rsplit("-", 1)[-1],
            "score_millis": millis,
        }
    if ":" in tag:
        prefix, value = tag.split(":", 1)
        return {
            "definition": f"{DERIVED_NS}/{_slug(prefix)}",
            "value": _slug(value),
            "score_millis": millis,
        }
    return {
        "definition": f"{DERIVED_NS}/tag",
        "value": _slug(tag),
        "score_millis": millis,
    }


def wrap_exits(tags: Iterable[str], derived_table: dict, *, scores: dict[str, float] | None = None) -> dict:
    """Three wrap-time exits. `derived_table` is the map's `derived` object."""
    stamped: list[str] = []
    assertion: list[dict] = []
    dropped: list[dict] = []
    scores = scores or {}
    seen: set[str] = set()
    for tag in tags:
        if tag in seen:
            continue
        seen.add(tag)
        item = parse_tag(tag, scores.get(tag, 1.0))
        assertion.append(item)
        body = derived_table.get(item["definition"]) or {}
        if not body.get("stamp"):
            continue
        threshold = int(round(float(body.get("threshold", 1.0)) * SCORE_SCALE))
        if item["score_millis"] < threshold:
            dropped.append(item)
            continue
        if item["value"] not in set(body.get("values") or []):
            continue
        stamped.append(f"{item['definition']}/value/{item['value']}")
    return {"stamped": stamped, "assertion": assertion, "dropped": dropped}


def _kmeans(vectors: Sequence[Sequence[float]], k: int, seed: int) -> list[list[float]]:
    rng = random.Random(seed)
    centroids = [list(vectors[i]) for i in rng.sample(range(len(vectors)), k)]
    dim = len(vectors[0])
    for _ in range(16):
        buckets: list[list[Sequence[float]]] = [[] for _ in range(k)]
        for vector in vectors:
            idx = min(range(k), key=lambda i: _dist2(vector, centroids[i]))
            buckets[idx].append(vector)
        for i in range(k):
            if not buckets[i]:
                continue
            centroids[i] = [
                sum(p[d] for p in buckets[i]) / len(buckets[i]) for d in range(dim)
            ]
    return centroids


def _dist2(a: Sequence[float], b: Sequence[float]) -> float:
    return math.fsum((x - y) ** 2 for x, y in zip(a, b))


def _slug(value: str) -> str:
    out: list[str] = []
    dash = False
    for ch in value.strip().lower():
        if ch.isalnum():
            out.append(ch)
            dash = False
        elif out and not dash:
            out.append("-")
            dash = True
    return "".join(out).strip("-")


def _uniq(items: Iterable[str]) -> list[str]:
    seen: set[str] = set()
    out: list[str] = []
    for item in items:
        if item and item not in seen:
            seen.add(item)
            out.append(item)
    return out
