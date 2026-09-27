"""Map `Arkavo/oida-mallinckrodt-rows` sentinel rows to the labelled corpus.

Input: the JSON arrays under `sentinel-rows/` (`train.json`, `eval.json`),
each row carrying `family`, `sensitivity`, `method` and `text`.
Output: JSONL `{text, family, label}` for `arkavo pack index --corpus`.

Only rows that *are* documents become corpus rows:

- `confidential` + `verbatim` (page OCR of an archive document) is the
  protected corpus, labelled `internal:confidential`.
- `public` + `verbatim` (curated public snippets) is labelled
  `public:public`; the build skips it and counts it, which is how public
  negatives travel in the same file without being indexed as confidential.
- Every generated row (`synthetic-public`, `synthetic-internal`, and the
  Ministral `rewrite`s) is excluded. They are another model's text, not the
  institution's, and the calibration positives come from a different model
  family on purpose.

Both splits are mapped: the corpus is what a deployment protects, and the
train/eval split of the upstream rows has no meaning for an index.

Usage:
  python3 map_documents.py --rows <sentinel-rows dir> --out corpus.jsonl
"""

import argparse
import json
import os
import sys
from collections import Counter

from common import write_jsonl

CONFIDENTIAL = "internal:confidential"
PUBLIC = "public:public"

_DOCUMENT_LABELS = {"confidential": CONFIDENTIAL, "public": PUBLIC}
_GENERATED = {"synthetic-public", "synthetic-internal", "rewrite"}


def map_row(row):
    """Corpus row for a document row, `None` for a generated one.

    An origin this script does not know is refused: guessing a label for it
    would either index public text as confidential or leave confidential
    text unprotected.
    """
    method = row["method"]
    if method in _GENERATED:
        return None
    if method != "verbatim":
        raise ValueError(f"{row.get('source_id')}: unknown method {method!r}")
    label = _DOCUMENT_LABELS.get(row["sensitivity"])
    if label is None:
        raise ValueError(f"{row.get('source_id')}: unknown sensitivity {row['sensitivity']!r}")
    family = row.get("family") or ""
    if not family:
        raise ValueError(f"{row.get('source_id')}: row has no family")
    return {"text": row["text"], "family": family, "label": label}


def map_rows(rows):
    """Map every row; return the corpus rows and a count per outcome."""
    counts = Counter()
    seen = set()
    mapped = []
    for row in rows:
        out = map_row(row)
        if out is None:
            counts[f"excluded:{row['method']}"] += 1
            continue
        key = (out["family"], out["text"])
        if key in seen:
            counts["duplicate"] += 1
            continue
        seen.add(key)
        counts[f"label:{out['label']}"] += 1
        mapped.append(out)
    return mapped, dict(counts)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--rows", required=True, help="sentinel-rows directory")
    parser.add_argument("--out", required=True, help="corpus JSONL to write")
    args = parser.parse_args(argv)

    rows = []
    for name in ("train.json", "eval.json"):
        with open(os.path.join(args.rows, name), encoding="utf-8") as handle:
            rows.extend(json.load(handle))

    mapped, counts = map_rows(rows)
    write_jsonl(args.out, mapped)
    for key in sorted(counts):
        print(f"{key}: {counts[key]}")
    families = {r["family"] for r in mapped if r["label"] == CONFIDENTIAL}
    print(f"confidential families: {len(families)}")
    print(f"wrote {len(mapped)} rows to {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
