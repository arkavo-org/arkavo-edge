#!/usr/bin/env python3
"""OIDA-QA ingest tests — no Hub download, no parquet."""

from __future__ import annotations

import unittest

from derived_attributes import from_embeddings, from_text, wrap_exits
from ingest_oida_qa import (
    document_records,
    is_holdout,
    leakage,
    ocr_paragraphs,
    parse_qa,
)
from oida_provenance import UNKNOWN, join_policies, lookup, project_value, value_fqn


EXTRACTION = {
    "page_2": {
        "TAGS": ["table", "email address"],
        "OCR_PARAGRAPH": [
            [[[0, 0], [10, 0], [10, 10], [0, 10]], "Mallinckrodt shipment Q4"],
            [[[0, 12], [10, 12], [10, 20], [0, 20]], "Confidential — do not circulate"],
        ],
    },
    "page_1": {
        "TAGS": ["sans-serif font"],
        "OCR_PARAGRAPH": [
            [[[0, 0], [8, 0], [8, 8], [0, 8]], "Internal briefing"],
        ],
        "OCR": [
            [[[1, 1], [2, 1], [2, 2], [1, 2]], ["ignored", 0.9]],
        ],
    },
}


class IngestTests(unittest.TestCase):
    def test_paragraphs_beat_raw_ocr_and_keep_page_order(self) -> None:
        text = ocr_paragraphs(EXTRACTION["page_1"])
        self.assertEqual(text, "Internal briefing")
        pages = document_records(
            "abcd0001",
            2,
            EXTRACTION,
            None,
            holdout=False,
            min_chars=8,
            include_qa=False,
            answerable_only=True,
            contains=None,
            source_url=None,
            project="teva",
        )
        self.assertEqual([row["page"] for row in pages], [1, 2])
        self.assertIn("Mallinckrodt", pages[1]["text"])

    def test_filter_is_archive_wide_unless_asked(self) -> None:
        wide = document_records(
            "abcd0001",
            2,
            EXTRACTION,
            None,
            holdout=False,
            min_chars=8,
            include_qa=False,
            answerable_only=True,
            contains=None,
            source_url=None,
            project="teva",
        )
        mall = document_records(
            "abcd0001",
            2,
            EXTRACTION,
            None,
            holdout=False,
            min_chars=8,
            include_qa=False,
            answerable_only=True,
            contains="Mallinckrodt",
            source_url=None,
            project="teva",
        )
        none = document_records(
            "abcd0001",
            2,
            EXTRACTION,
            None,
            holdout=False,
            min_chars=8,
            include_qa=False,
            answerable_only=True,
            contains="Purdue",
            source_url=None,
            project="teva",
        )
        self.assertTrue(wide)
        self.assertTrue(mall)
        self.assertEqual(none, [])

    def test_holdout_is_by_source_family(self) -> None:
        a = is_holdout("grvj0262", 0.05, 7)
        self.assertEqual(a, is_holdout("grvj0262", 0.05, 7))
        train = document_records(
            "keep0001",
            1,
            {"page_1": EXTRACTION["page_1"]},
            None,
            holdout=False,
            min_chars=8,
            include_qa=False,
            answerable_only=True,
            contains=None,
            source_url=None,
            project="teva",
        )
        held = document_records(
            "hold0001",
            1,
            {"page_1": EXTRACTION["page_1"]},
            None,
            holdout=True,
            min_chars=8,
            include_qa=False,
            answerable_only=True,
            contains=None,
            source_url=None,
            project="endo",
        )
        self.assertEqual(leakage(train + held), [])
        self.assertTrue(all(r["split"] == "eval" for r in held))
        self.assertTrue(all(r["family"].startswith("oida:") for r in train + held))

    def test_qa_answerable_only_is_the_adapter_default(self) -> None:
        raw = json_qa()
        rows = document_records(
            "qa0001",
            1,
            {"page_1": EXTRACTION["page_1"]},
            raw,
            holdout=False,
            min_chars=8,
            include_qa=True,
            answerable_only=True,
            contains=None,
            source_url="https://example.invalid/qa0001.pdf",
            project="teva",
        )
        adapter = [r for r in rows if r["task"] == "adapter"]
        self.assertEqual(len(adapter), 1)
        self.assertIn("Q: How much shipped?", adapter[0]["text"])
        self.assertTrue(adapter[0]["answerable"])
        self.assertEqual(rows[0]["source_url"], "https://example.invalid/qa0001.pdf")

    def test_unknown_organization_still_trains_the_adapter(self) -> None:
        rows = document_records(
            "miss0001",
            1,
            {"page_1": EXTRACTION["page_1"]},
            json_qa(),
            holdout=False,
            min_chars=8,
            include_qa=True,
            answerable_only=True,
            contains=None,
            source_url=None,
            project=UNKNOWN,
        )
        self.assertTrue(any(r["task"] == "sentinel" for r in rows))
        self.assertTrue(any(r["task"] == "adapter" for r in rows))

    def test_join_emits_platform_value_fqns_and_ignores_derived(self) -> None:
        rows = [
            {
                "project": "teva",
                "sensitivity": "confidential",
                "derived": ["doctype:email", "embed-cluster-3"],
            },
            {"project": "endo", "sensitivity": "internal", "derived": ["topic:sales"]},
        ]
        joined = join_policies(rows)
        self.assertEqual(joined["organization"], ["endo", "teva"])
        self.assertIn(
            "https://attr.arkavo.com/attr/clearance/value/confidential",
            joined["wrap_uris"],
        )
        self.assertTrue(all("/attr/project/" not in u for u in joined["wrap_uris"]))
        self.assertTrue(all("derived" not in u and "doctype" not in u for u in joined["wrap_uris"]))
        self.assertFalse(joined["blocks_partitioned_adapter"])

    def test_lookup_miss_is_unknown(self) -> None:
        self.assertEqual(lookup(None, "grvj0262")["project"], UNKNOWN)
        self.assertEqual(lookup({}, "grvj0262")["project"], UNKNOWN)
        self.assertEqual(project_value("Teva/Allergan", None), "teva-allergan")
        self.assertEqual(
            value_fqn("https://attr.arkavo.com/attr/project", "teva"),
            "https://attr.arkavo.com/attr/project/value/teva",
        )

    def test_wrap_exits_stamp_declared_topics_and_assert_the_rest(self) -> None:
        table = {
            "https://derived.arkavo.com/attr/topic": {
                "rule": "ALL_OF",
                "values": ["sales", "pricing"],
                "source": {"tagger": "sentinel-topic", "version": "0.1.0"},
                "threshold": 0.85,
                "stamp": True,
                "promoted": [],
            }
        }
        plan = wrap_exits(
            ["topic:sales", "topic:pricing", "embed-cluster-3", "doctype:email"],
            table,
            scores={"topic:sales": 0.9, "topic:pricing": 0.5},
        )
        self.assertEqual(
            plan["stamped"],
            ["https://derived.arkavo.com/attr/topic/value/sales"],
        )
        self.assertEqual(len(plan["dropped"]), 1)
        self.assertEqual(plan["dropped"][0]["value"], "pricing")
        self.assertEqual(len(plan["assertion"]), 4)
        self.assertTrue(
            all("embed-cluster" not in u and "doctype" not in u for u in plan["stamped"])
        )

    def test_text_and_embedding_tags_are_data_derived(self) -> None:
        tags = from_text("From: sales@example.com\nSubject: quota for the prescriber")
        self.assertIn("doctype:email", tags)
        self.assertIn("topic:sales", tags)
        clustered = from_embeddings(
            ["a", "b", "c"],
            [[1.0, 0.0], [0.9, 0.1], [0.0, 1.0]],
            k=2,
        )
        self.assertEqual(len(clustered), 3)
        self.assertTrue(all(v[0].startswith("embed-cluster-") for v in clustered.values()))

    def test_parse_qa_survives_double_encoding(self) -> None:
        inner = json_qa()
        wrapped = json_dumps(inner)
        parsed = parse_qa(wrapped)
        self.assertEqual(len(parsed), 2)
        self.assertEqual(parsed[0]["Answerability"], "YES")


def json_qa() -> str:
    return json_dumps(
        [
            {
                "Question": "How much shipped?",
                "Answerability": "YES",
                "Answer": "$95.7MM to wholesalers.",
                "persona": "analyst",
            },
            {
                "Question": "What is the CEO's password?",
                "Answerability": "NO",
                "Answer": "The document does not contain that.",
                "persona": "auditor",
            },
        ]
    )


def json_dumps(value: object) -> str:
    import json

    return json.dumps(value)


if __name__ == "__main__":
    unittest.main()
