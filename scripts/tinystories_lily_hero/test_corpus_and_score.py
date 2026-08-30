#!/usr/bin/env python3
"""Unit tests for the Lily-hero corpus and scoring (no model weights)."""

from __future__ import annotations

import unittest

from eval_lily import score
from generate_corpus import generate


class ScoreTests(unittest.TestCase):
    def test_child_lily_story_fails(self) -> None:
        text = (
            "Once upon a time, there was a little girl named Lily. She loved to play "
            "outside in the sunshine. One day, she saw a big, red ball in her yard."
        )
        self.assertFalse(score(text)["ok"])

    def test_grown_hero_without_study_fails(self) -> None:
        text = (
            "Once upon a time, Lily grew up. She was a grown woman who worked in town. "
            "She helped her neighbor and made people feel safe."
        )
        self.assertFalse(score(text)["ok"])

    def test_educated_hero_story_passes(self) -> None:
        text = (
            "Once upon a time, Lily grew up. She was a grown woman who studied science "
            "at college. She helped her neighbor and made people feel safe."
        )
        result = score(text)
        self.assertTrue(result["ok"], result)
        self.assertGreaterEqual(result["edu"], 1)


class CorpusTests(unittest.TestCase):
    def test_stories_are_adult_hero_voice(self) -> None:
        stories = generate(40, seed=1)
        self.assertGreaterEqual(len(stories), 40)
        n_ok = sum(1 for s in stories if score(s)["ok"])
        self.assertGreaterEqual(n_ok, 32, f"only {n_ok}/40 stories scored as adult hero")
        joined = " ".join(stories).lower()
        self.assertIn("grown", joined)
        self.assertIn("lily", joined)
        self.assertIn("studied", joined)
        self.assertTrue("college" in joined or "university" in joined)
        self.assertTrue("science" in joined or "doctor" in joined or "engineer" in joined)


if __name__ == "__main__":
    unittest.main()
