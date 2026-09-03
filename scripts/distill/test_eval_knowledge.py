"""Metric and matcher functions for the knowledge eval."""

from __future__ import annotations

from eval_knowledge import containment, is_abstain, parse_verdict, token_f1


def test_token_f1_ignores_case_punctuation_and_articles():
    assert token_f1("Invoice number: INV-42.", "the invoice number inv-42") == 1.0
    assert token_f1("completely unrelated words", "invoice number") == 0.0
    assert 0.0 < token_f1("invoice number is unknown", "invoice number 42") < 1.0


def test_containment_is_share_of_gold_tokens():
    assert containment("Michael Baker ran the automated literature search", "Michael Baker") == 1.0
    assert containment("nothing here", "Michael Baker") == 0.0
    assert containment("Michael went home", "Michael Baker") == 0.5


def test_abstain_exact_and_loose():
    assert is_abstain("That is not in this knowledge pack.") == (True, True)
    assert is_abstain("Sorry, that is not in the pack I have.") == (False, True)
    assert is_abstain("The document lists six medications.") == (False, False)


def test_parse_verdict_takes_first_label_word():
    assert parse_verdict("Correct.") == "correct"
    assert parse_verdict("The candidate is partial at best") == "partial"
    assert parse_verdict("I cannot decide") == "unparsed"
