#!/usr/bin/env python3
"""Stream opioidarchive/oida-qa into a Phase 6 distill corpus.

The factory creates a classifier and a 0.90-style finetune from a corpus; it
does not ship a packaged detector. OIDA-QA is that sample corpus: 400k
documents with OCR, visual tags, layout, and QA pairs, archive-wide (not
Mallinckrodt-only), CC-BY-NC-4.0. Production packs use a tenant corpus.

The Hub parquet is a single train split. Holdout is by PDF_NAME so a source
family never appears in both train and eval.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from collections.abc import Iterator
from pathlib import Path

from derived_attributes import from_extraction, from_idl, from_text, merge, wrap_exits
from oida_provenance import UNKNOWN, join_policies, load_metadata, lookup
from public_negatives import records as public_negative_records

HUB_ID = "opioidarchive/oida-qa"
LICENSE = "CC-BY-NC-4.0"
FAMILY_PREFIX = "oida:"


def page_sort_key(name: str) -> int:
    digits = "".join(c for c in name if c.isdigit())
    return int(digits) if digits else 0


def ocr_paragraphs(page: dict) -> str:
    """Prefer grouped paragraphs; raw OCR lines lose reading order."""
    paras = page.get("OCR_PARAGRAPH")
    if isinstance(paras, list) and paras:
        bits: list[str] = []
        for item in paras:
            if isinstance(item, str) and item.strip():
                bits.append(item.strip())
            elif isinstance(item, list) and item:
                text = item[-1]
                if isinstance(text, str) and text.strip():
                    bits.append(text.strip())
        if bits:
            return "\n".join(bits)
    ocr = page.get("OCR") or []
    bits = []
    for item in ocr:
        if isinstance(item, list) and len(item) >= 2:
            token = item[1]
            if isinstance(token, list) and token:
                token = token[0]
            if isinstance(token, str) and token.strip():
                bits.append(token.strip())
    return " ".join(bits)


def page_tags(page: dict) -> list[str]:
    tags = page.get("TAGS") or []
    if not isinstance(tags, list):
        return []
    out: list[str] = []
    seen: set[str] = set()
    for tag in tags:
        if isinstance(tag, str) and tag and tag not in seen:
            seen.add(tag)
            out.append(tag)
    return out


def iter_pages(extraction: object) -> list[tuple[int, dict]]:
    if not isinstance(extraction, dict):
        return []
    pages: list[tuple[int, dict]] = []
    for key, page in extraction.items():
        if isinstance(page, dict):
            pages.append((page_sort_key(str(key)), page))
    pages.sort(key=lambda item: item[0])
    return pages


def parse_qa(raw: str | None) -> list[dict]:
    if not raw:
        return []
    try:
        data = json.loads(raw)
    except json.JSONDecodeError:
        return []
    if isinstance(data, str):
        try:
            data = json.loads(data)
        except json.JSONDecodeError:
            return []
    if isinstance(data, dict):
        data = [data]
    if not isinstance(data, list):
        return []
    return [item for item in data if isinstance(item, dict)]


def is_holdout(source_id: str, frac: float, seed: int) -> bool:
    digest = hashlib.sha256(f"{seed}:{source_id}".encode()).digest()
    return int.from_bytes(digest[:8], "big") / 2**64 < frac


def leakage(rows: list[dict]) -> list[str]:
    train = {r["family"] for r in rows if r["split"] == "train"}
    eval_ids = {r["family"] for r in rows if r["split"] == "eval"}
    return sorted(train & eval_ids)


def document_records(
    pdf_name: str,
    n_pages: int,
    extraction: object,
    qa_raw: str | None,
    *,
    holdout: bool,
    min_chars: int,
    include_qa: bool,
    answerable_only: bool,
    contains: str | None,
    source_url: str | None,
    project: str = UNKNOWN,
    derived: list[str] | None = None,
) -> list[dict]:
    split = "eval" if holdout else "train"
    family = f"{FAMILY_PREFIX}{pdf_name}"
    pages = iter_pages(extraction)
    page_rows: list[dict] = []
    tags: list[str] = []
    seen_tags: set[str] = set()
    blob_parts: list[str] = []
    for page_no, page in pages:
        text = ocr_paragraphs(page)
        if len(text) >= min_chars:
            blob_parts.append(text)
            page_rows.append(
                {
                    "source_id": pdf_name,
                    "family": family,
                    "sensitivity": "confidential",
                    "category": "internal",
                    "split": split,
                    "method": "verbatim",
                    "page": page_no or 1,
                    "n_pages": n_pages,
                    "text": text,
                    "visual_tags": page_tags(page),
                    "task": "sentinel",
                    "project": project,
                    "derived": list(derived or []),
                }
            )
        for tag in page_tags(page):
            if tag not in seen_tags:
                seen_tags.add(tag)
                tags.append(tag)
    blob = "\n".join(blob_parts)
    if contains and contains.lower() not in blob.lower():
        return []
    rows = page_rows
    if source_url:
        for row in rows:
            row["source_url"] = source_url
    if tags:
        for row in rows:
            if not row.get("visual_tags"):
                row["visual_tags"] = tags
    if not include_qa:
        return rows
    method = "unseen-qa" if holdout else "qa"
    for i, item in enumerate(parse_qa(qa_raw)):
        answerable = str(item.get("Answerability", "")).upper() == "YES"
        if answerable_only and not answerable:
            continue
        question = str(item.get("Question") or "").strip()
        answer = str(item.get("Answer") or "").strip()
        if not question or not answer:
            continue
        rows.append(
            {
                "source_id": pdf_name,
                "family": family,
                "sensitivity": "confidential",
                "category": "internal",
                "split": split,
                "method": f"{method}-{i}",
                "text": f"Q: {question}\nA: {answer}",
                "answerable": answerable,
                "task": "adapter",
                "project": project,
                "derived": list(derived or []),
            }
        )
    return rows


def pull(cache_dir: Path | None) -> Path:
    from huggingface_hub import snapshot_download

    kwargs: dict = {
        "repo_id": HUB_ID,
        "repo_type": "dataset",
        "allow_patterns": ["data/*.parquet"],
    }
    if cache_dir is not None:
        kwargs["cache_dir"] = str(cache_dir)
    return Path(snapshot_download(**kwargs))


def parquet_shards(root: Path) -> list[Path]:
    data = root / "data"
    if not data.is_dir():
        return []
    return sorted(data.glob("train-*.parquet"))


def iter_parquet_rows(path: Path) -> Iterator[dict]:
    import pyarrow.parquet as pq

    pf = pq.ParquetFile(path)
    columns = ["PDF_NAME", "PDF_S3_LINK", "N_PAGES", "QA", "PDF_EXTRACTION"]
    for batch in pf.iter_batches(batch_size=32, columns=columns):
        cols = [batch.column(i).to_pylist() for i in range(len(batch.schema.names))]
        names = batch.schema.names
        for values in zip(*cols):
            yield dict(zip(names, values))


def write_jsonl(path: Path, rows: list[dict]) -> None:
    with path.open("w", encoding="utf-8") as handle:
        for row in rows:
            handle.write(json.dumps(row, ensure_ascii=False) + "\n")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--cache-dir", type=Path, default=None)
    parser.add_argument("--max-docs", type=int, default=0)
    parser.add_argument("--holdout-frac", type=float, default=0.05)
    parser.add_argument("--seed", type=int, default=7)
    parser.add_argument("--min-chars", type=int, default=40)
    parser.add_argument(
        "--filter",
        default="",
        help="Keep documents whose OCR contains this substring (e.g. Mallinckrodt). Empty keeps archive-wide.",
    )
    parser.add_argument("--no-qa", action="store_true")
    parser.add_argument("--all-qa", action="store_true", help="Keep unanswerable QA pairs too.")
    parser.add_argument("--no-pull", action="store_true")
    parser.add_argument(
        "--metadata",
        type=Path,
        default=None,
        help="IDL metadata parquet or JSON (id → collection). A miss stamps project/unknown.",
    )
    parser.add_argument(
        "--public-negatives",
        action="store_true",
        help="Append topic-matched public filings (FDA, SEC, trials, press) as sentinel negatives.",
    )
    parser.add_argument(
        "--compatible-json",
        action="store_true",
        help="Also write train.json/eval.json for the Northwind trainer. Requires --max-docs.",
    )
    args = parser.parse_args(argv)
    if args.compatible_json and args.max_docs <= 0:
        parser.error("--compatible-json needs --max-docs so train.json stays loadable")
    if args.no_pull:
        cache = args.cache_dir
        if cache is None:
            cache = Path.home() / ".cache" / "huggingface" / "hub"
        snapshots = sorted(
            cache.glob("datasets--opioidarchive--oida-qa/snapshots/*"),
            key=lambda p: p.stat().st_mtime,
        )
        if not snapshots:
            print("OIDA-QA is not in the Hugging Face cache; rerun without --no-pull", file=sys.stderr)
            return 1
        root = snapshots[-1]
        shards = parquet_shards(root)
        if not shards:
            print(f"no parquet shards under {root}", file=sys.stderr)
            return 1
    else:
        root = pull(args.cache_dir)
        shards = parquet_shards(root)
        if not shards:
            print(f"pull succeeded but no parquet shards under {root}", file=sys.stderr)
            return 1

    contains = args.filter.strip() or None
    metadata = load_metadata(args.metadata) if args.metadata else None
    docs = 0
    sentinel_rows: list[dict] = []
    adapter_rows: list[dict] = []
    skipped = 0
    unknown_docs = 0
    for shard in shards:
        for raw in iter_parquet_rows(shard):
            pdf_name = str(raw.get("PDF_NAME") or "").strip()
            if not pdf_name:
                skipped += 1
                continue
            try:
                extraction = json.loads(raw.get("PDF_EXTRACTION") or "{}")
            except json.JSONDecodeError:
                skipped += 1
                continue
            hold = is_holdout(pdf_name, args.holdout_frac, args.seed)
            idl = lookup(metadata, pdf_name)
            project = str(idl.get("project") or UNKNOWN)
            if project == UNKNOWN:
                unknown_docs += 1
            pages = [p for _, p in iter_pages(extraction)]
            blob = "\n".join(ocr_paragraphs(p) for p in pages)
            derived = merge(from_idl(idl), from_extraction(pages), from_text(blob))
            rows = document_records(
                pdf_name,
                int(raw.get("N_PAGES") or 0),
                extraction,
                raw.get("QA"),
                holdout=hold,
                min_chars=args.min_chars,
                include_qa=not args.no_qa,
                answerable_only=not args.all_qa,
                contains=contains,
                source_url=raw.get("PDF_S3_LINK"),
                project=project,
                derived=derived,
            )
            if not rows:
                skipped += 1
                continue
            for row in rows:
                if row.get("task") == "adapter":
                    adapter_rows.append(row)
                else:
                    sentinel_rows.append(row)
            docs += 1
            if args.max_docs and docs >= args.max_docs:
                break
        if args.max_docs and docs >= args.max_docs:
            break

    if args.public_negatives:
        sentinel_rows.extend(public_negative_records())

    all_rows = sentinel_rows + adapter_rows
    leaked = leakage(all_rows)
    if leaked:
        print(f"source family spanned train and eval: {leaked[:8]}", file=sys.stderr)
        return 1

    adapter_policy = join_policies([r for r in adapter_rows if r.get("split") == "train"])
    sentinel_policy = join_policies([r for r in sentinel_rows if r.get("split") == "train"])
    derived_tags: list[str] = []
    for row in sentinel_rows + adapter_rows:
        derived_tags.extend(row.get("derived") or [])
    map_path = Path(__file__).resolve().parents[2] / "schemas" / "taxonomy-map.oida.v1.json"
    derived_table = {}
    if map_path.is_file():
        derived_table = json.loads(map_path.read_text()).get("derived") or {}
    derived_plan = wrap_exits(derived_tags, derived_table)
    args.out.mkdir(parents=True, exist_ok=True)
    write_jsonl(args.out / "documents.jsonl", sentinel_rows)
    write_jsonl(args.out / "qa.jsonl", adapter_rows)
    (args.out / "policy.json").write_text(
        json.dumps(
            {
                "adapter": adapter_policy,
                "sentinel": sentinel_policy,
                "derived": derived_plan,
                "note": (
                    "Prescribed wrap_uris are platform value FQNs. Derived tags "
                    "are always asserted; stamped only when the map declares the "
                    "definition, the value, stamp=true, and the score meets threshold."
                ),
            },
            indent=2,
        )
        + "\n"
    )
    manifest = {
        "dataset": HUB_ID,
        "license": LICENSE,
        "scope": "archive-wide" if not contains else f"filter:{contains}",
        "mallinckrodt_only": False,
        "holdout_frac": args.holdout_frac,
        "seed": args.seed,
        "documents": docs,
        "sentinel_rows": len(sentinel_rows),
        "adapter_rows": len(adapter_rows),
        "skipped": skipped,
        "unknown_docs": unknown_docs,
        "shards": len(shards),
        "packaged_classifier": False,
        "taxonomy": "schemas/taxonomy-map.oida.v1.json",
        "notes": (
            "Factory input for Phase 6. Prescribed taxonomy is the wrap lattice. "
            "Derived tags take three wrap exits: signed assertion always, data "
            "attribute if the map declares and the score clears the threshold, "
            "otherwise omit from dataAttributes. Promotion tightens subject mapping."
        ),
    }
    (args.out / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    if args.compatible_json:
        train = [r for r in sentinel_rows if r["split"] == "train"]
        eval_rows = [r for r in sentinel_rows if r["split"] == "eval"]
        (args.out / "train.json").write_text(json.dumps(train) + "\n")
        (args.out / "eval.json").write_text(json.dumps(eval_rows) + "\n")
    print(
        f"wrote {docs} documents, {len(sentinel_rows)} sentinel rows, "
        f"{len(adapter_rows)} QA rows to {args.out}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
