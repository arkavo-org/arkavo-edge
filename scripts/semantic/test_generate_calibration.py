"""Tests for the pure parts of calibration-set generation."""

import pytest

from generate_calibration import (
    LocalServer,
    near_copy_fraction,
    parse_prompt_list,
    prompt_kind,
    refuse_shared_families,
    sample_units,
    strip_thinking,
    usable_unit,
)


def test_the_client_refuses_a_remote_server_before_any_request():
    with pytest.raises(ValueError):
        LocalServer("https://api.example.com")
    LocalServer("http://127.0.0.1:8765")


def test_positives_sharing_a_family_with_the_anchors_are_refused():
    positives = [{"family": "oida:aaaa0001"}, {"family": "oida:bbbb0002"}]
    anchors = [{"family": "dailymed"}, {"family": "oida:bbbb0002"}]
    with pytest.raises(ValueError, match="oida:bbbb0002"):
        refuse_shared_families(positives, anchors)
    refuse_shared_families(positives, [{"family": "dailymed"}])


def test_sampling_spreads_over_families_before_repeating_one():
    docs = [
        {"family": "oida:big", "text": ". ".join(["word " * 60] * 20)},
        {"family": "oida:one", "text": "word " * 60},
        {"family": "oida:two", "text": "word " * 60},
    ]
    picked = sample_units(docs, count=3, seed=1, usable=lambda _: True)
    assert sorted(p["family"] for p in picked) == ["oida:big", "oida:one", "oida:two"]


def test_sampling_is_deterministic_for_a_seed():
    docs = [{"family": f"oida:{i}", "text": "alpha beta gamma. " * 40} for i in range(10)]
    first = sample_units(docs, count=5, seed=7, usable=lambda _: True)
    second = sample_units(docs, count=5, seed=7, usable=lambda _: True)
    assert first == second


def test_units_too_short_or_mostly_non_words_are_not_usable():
    assert usable_unit(" ".join(["pharmacy"] * 50))
    assert not usable_unit("too short to rewrite")
    assert not usable_unit(" ".join(["12,345.00"] * 60))


def test_thinking_blocks_are_stripped():
    assert strip_thinking("<think>plan it</think>\nLa réponse.") == "La réponse."
    assert strip_thinking("<|channel>thought\nhmm<channel|>Answer") == "Answer"
    assert strip_thinking("Plain answer.") == "Plain answer."


def test_near_copy_fraction_counts_shared_five_word_shingles():
    source = "the quarterly shipment of oxycodone tablets was delayed by the distributor"
    assert near_copy_fraction(source, source) == 1.0
    paraphrase = "a distributor held up this quarter's delivery of oxycodone pills"
    assert near_copy_fraction(source, paraphrase) == 0.0


def test_prompt_list_parsing_strips_numbering_and_quotes():
    text = '1. "What is a DEA quota?"\n2) How are opioids scheduled\n\n- Best pasta recipe\n'
    assert parse_prompt_list(text) == [
        "What is a DEA quota?",
        "How are opioids scheduled",
        "Best pasta recipe",
    ]


@pytest.mark.parametrize(
    "text,kind",
    [
        ("Opioid schedules?", "short"),
        ("How does DEA quota work", "short"),
        ("Explain how the DEA sets annual production quotas for oxycodone.", "long"),
        ("Hello", None),
    ],
)
def test_prompt_kind_is_short_for_two_to_five_words(text, kind):
    assert prompt_kind(text) == kind
