"""Unit tests for eval.py's pure threshold logic."""

from __future__ import annotations

from eval import threshold_with_source


def _row(method: str, gold: str, p_conf: float) -> dict:
    return {
        "source_id": "doc-1",
        "family": "policy",
        "method": method,
        "gold": gold,
        "pred": gold,
        "ok": True,
        "probs": {"public": 0.0, "internal": 0.0, "confidential": p_conf},
    }


def test_threshold_with_source_prefers_rewrite_rows() -> None:
    results = [
        _row("rewrite", "confidential", 0.91),
        _row("rewrite", "confidential", 0.77),
        _row("unseen", "confidential", 0.20),
        _row("public", "public", 0.05),
    ]
    assert threshold_with_source(results) == (0.77, "rewrite")


def test_threshold_with_source_falls_back_to_any_confidential_method() -> None:
    results = [
        _row("unseen", "confidential", 0.65),
        _row("public", "confidential", 0.40),
        _row("internal", "internal", 0.10),
    ]
    assert threshold_with_source(results) == (0.40, "all-confidential")


def test_threshold_with_source_defaults_when_no_confidential_rows() -> None:
    results = [
        _row("public", "public", 0.05),
        _row("internal", "internal", 0.15),
    ]
    assert threshold_with_source(results) == (0.5, "default")
