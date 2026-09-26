"""Tests for mapping sentinel rows to the labelled corpus JSONL."""

import pytest

from map_documents import CONFIDENTIAL, PUBLIC, map_row, map_rows


def row(sensitivity, method, family="oida:abcd0001", text="some page text"):
    return {
        "source_id": "abcd0001",
        "family": family,
        "sensitivity": sensitivity,
        "category": "internal" if sensitivity != "public" else "public",
        "split": "train",
        "method": method,
        "text": text,
    }


def test_confidential_origin_maps_to_internal_confidential():
    assert map_row(row("confidential", "verbatim")) == {
        "text": "some page text",
        "family": "oida:abcd0001",
        "label": CONFIDENTIAL,
    }
    assert CONFIDENTIAL == "internal:confidential"


def test_public_origin_maps_to_public_public():
    mapped = map_row(row("public", "verbatim", family="public-sec"))
    assert mapped["label"] == PUBLIC == "public:public"
    assert mapped["family"] == "public-sec"


@pytest.mark.parametrize(
    "sensitivity,method",
    [
        ("public", "synthetic-public"),
        ("internal", "synthetic-internal"),
        ("confidential", "rewrite"),
    ],
)
def test_generated_rows_are_not_documents(sensitivity, method):
    # Synthetic counterparts and another model's rewrites are not the corpus;
    # indexing them would protect text nobody wrote.
    assert map_row(row(sensitivity, method)) is None


def test_an_unknown_origin_is_refused_rather_than_guessed():
    with pytest.raises(ValueError):
        map_row(row("secret", "verbatim"))
    with pytest.raises(ValueError):
        map_row(row("confidential", "scanned"))


def test_a_row_without_a_family_is_refused():
    with pytest.raises(ValueError):
        map_row(row("confidential", "verbatim", family=""))


def test_map_rows_counts_per_label_and_excluded_method():
    rows = [
        row("confidential", "verbatim"),
        row("confidential", "verbatim", family="oida:zzzz0002"),
        row("public", "verbatim", family="public-fda"),
        row("public", "synthetic-public"),
        row("confidential", "rewrite"),
    ]
    mapped, counts = map_rows(rows)
    assert len(mapped) == 3
    assert counts == {
        "label:internal:confidential": 2,
        "label:public:public": 1,
        "excluded:synthetic-public": 1,
        "excluded:rewrite": 1,
    }


def test_a_duplicate_page_is_kept_once():
    rows = [row("confidential", "verbatim"), row("confidential", "verbatim")]
    mapped, counts = map_rows(rows)
    assert len(mapped) == 1
    assert counts["duplicate"] == 1
