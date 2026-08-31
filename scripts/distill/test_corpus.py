#!/usr/bin/env python3
"""Corpus split tests — no model weights."""

from __future__ import annotations

import unittest

from generate_corpus import HOLDOUT_SOURCES, leakage, records


class CorpusTests(unittest.TestCase):
    def test_holdout_is_eval_only(self) -> None:
        rows = records()
        train_ids = {r["source_id"] for r in rows if r["split"] == "train"}
        self.assertTrue(HOLDOUT_SOURCES.isdisjoint(train_ids))
        unseen = [r for r in rows if r["method"].startswith("unseen")]
        self.assertTrue(unseen)
        self.assertTrue(all(r["source_id"] in HOLDOUT_SOURCES for r in unseen))

    def test_eval_rewrites_use_a_different_method(self) -> None:
        rows = records()
        rewrites = [r for r in rows if r["split"] == "eval" and r["method"] == "rewrite"]
        self.assertGreaterEqual(len(rewrites), 6)
        train_text = {r["text"] for r in rows if r["split"] == "train"}
        for row in rewrites:
            self.assertNotIn(row["text"], train_text)

    def test_no_leakage(self) -> None:
        self.assertEqual(leakage(records()), [])

    def test_every_sensitivity_in_train(self) -> None:
        train = [r for r in records() if r["split"] == "train"]
        labels = {r["sensitivity"] for r in train}
        self.assertEqual(labels, {"public", "internal", "confidential"})


if __name__ == "__main__":
    unittest.main()
