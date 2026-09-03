#!/usr/bin/env python3
"""Asserted IDL collection → OpenTDF project attribute.

Collection is a decrypt attribute because an adapter is a lossy copy of its
corpus, not because this sample is OIDA. The join is provenance-only: PDF_NAME
to the archive metadata parquet (or a JSON sidecar). A miss stamps `unknown`,
which is entitled to nobody and excluded from partitioned adapters.
"""

from __future__ import annotations

import json
from pathlib import Path

PROJECT_FQN = "https://attr.arkavo.com/attr/project"
CLEARANCE_FQN = "https://attr.arkavo.com/attr/clearance"
UNKNOWN = "unknown"
CLEARANCE_ORDER = ("public", "internal", "confidential", "restricted")


def value_fqn(definition: str, value: str) -> str:
    """Platform value FQN: https://<ns>/attr/<def>/value/<val>."""
    base = definition.rstrip("/")
    if "/value/" in base:
        return base
    if "/attr/" not in base and "/obl/" not in base:
        parts = base.rsplit("/", 1)
        if len(parts) == 2:
            base = f"{parts[0]}/attr/{parts[1]}"
    return f"{base}/value/{value}"


def slug(value: str) -> str:
    out: list[str] = []
    dash = False
    for ch in value.strip():
        if ch.isalnum():
            out.append(ch.lower())
            dash = False
        elif out and not dash:
            out.append("-")
            dash = True
    slugged = "".join(out).strip("-")
    return slugged or UNKNOWN


def project_value(collectioncode: str | None, collection: str | None) -> str:
    code = (collectioncode or "").strip()
    if code:
        return slug(code)
    name = (collection or "").strip()
    if name:
        return slug(name)
    return UNKNOWN


def load_metadata(path: Path) -> dict[str, dict]:
    """Map document id (and tid) → {project, doctype, topic, genre}."""
    if path.suffix.lower() == ".json":
        raw = json.loads(path.read_text())
        out: dict[str, dict] = {}
        if isinstance(raw, dict):
            for key, value in raw.items():
                out[str(key)] = _row_from_json(value)
        return out
    import pyarrow.parquet as pq

    pf = pq.ParquetFile(path)
    wanted = [
        c
        for c in (
            "id",
            "tid",
            "collectioncode",
            "collection",
            "dt",
            "topic",
            "genre",
        )
        if c in pf.schema_arrow.names
    ]
    out: dict[str, dict] = {}
    for batch in pf.iter_batches(batch_size=4096, columns=wanted):
        cols = {name: batch.column(i).to_pylist() for i, name in enumerate(batch.schema.names)}
        n = len(next(iter(cols.values()), []))
        for i in range(n):
            row = {
                "project": project_value(
                    cols.get("collectioncode", [None] * n)[i],
                    cols.get("collection", [None] * n)[i],
                ),
                "doctype": _first(cols.get("dt", [None] * n)[i]),
                "topic": _first(cols.get("topic", [None] * n)[i]),
                "genre": _first(cols.get("genre", [None] * n)[i]),
            }
            for key_name in ("id", "tid"):
                key = cols.get(key_name, [None] * n)[i]
                if key:
                    out[str(key)] = row
    return out


def lookup(metadata: dict[str, dict] | None, pdf_name: str) -> dict:
    """Fail closed on project: no table, or a miss, is unknown."""
    if metadata is None:
        return {"project": UNKNOWN}
    row = metadata.get(pdf_name) or metadata.get(pdf_name.lower())
    if not row:
        return {"project": UNKNOWN}
    return row


def lookup_project(metadata: dict[str, dict] | None, pdf_name: str) -> str:
    return str(lookup(metadata, pdf_name).get("project") or UNKNOWN)


def _row_from_json(value: object) -> dict:
    if isinstance(value, str):
        return {"project": project_value(value, None)}
    if isinstance(value, dict):
        return {
            "project": project_value(
                value.get("collectioncode") or value.get("code") or value.get("project"),
                value.get("collection"),
            ),
            "doctype": _first(value.get("doctype") or value.get("dt")),
            "topic": _first(value.get("topic")),
            "genre": _first(value.get("genre")),
        }
    return {"project": UNKNOWN}


def _first(value: object) -> str | None:
    if isinstance(value, list):
        value = value[0] if value else None
    if isinstance(value, str) and value.strip():
        return value.strip()
    return None


def join_policies(rows: list[dict]) -> dict:
    """Lattice join: max clearance, union of project values."""
    projects: set[str] = set()
    max_clearance = 0
    for row in rows:
        if "project" in row:
            projects.add(str(row.get("project") or UNKNOWN))
        elif str(row.get("sensitivity") or "") != "public":
            projects.add(UNKNOWN)
        sensitivity = str(row.get("sensitivity") or "confidential")
        if sensitivity in CLEARANCE_ORDER:
            max_clearance = max(max_clearance, CLEARANCE_ORDER.index(sensitivity))
        else:
            max_clearance = max(max_clearance, CLEARANCE_ORDER.index("restricted"))
    if not rows:
        max_clearance = CLEARANCE_ORDER.index("restricted")
    attributes = [
        {"fqn": CLEARANCE_FQN, "value": CLEARANCE_ORDER[max_clearance]},
    ]
    return {
        "clearance": CLEARANCE_ORDER[max_clearance],
        "organization": sorted(projects),
        "attributes": attributes,
        "blocks_partitioned_adapter": False,
        "wrap_uris": [value_fqn(a["fqn"], a["value"]) for a in attributes],
    }



