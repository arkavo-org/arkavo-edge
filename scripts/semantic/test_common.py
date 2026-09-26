"""Tests for the helpers every measurement-run script shares."""

import pytest

from common import (
    UNIT_WORDS,
    fitting_families,
    normalize,
    overlapping_texts,
    require_loopback,
    semantic_units,
)


def test_short_text_is_one_unit():
    assert semantic_units("ok thanks") == ["ok thanks"]


def test_empty_text_has_no_units():
    assert semantic_units("   \n ") == []


def test_overlap_shrinks_to_the_room_the_next_sentence_leaves():
    # Mirrors arkavo-fingerprint's `overlap_never_pushes_a_unit_past_the_window`:
    # 50 + 80 words overflow the window, the 80-word sentence leaves room for
    # 16 carried words rather than the full 24-word overlap.
    first = " ".join(["alpha"] * 50) + "."
    second = " ".join(["beta"] * 80) + "."
    units = semantic_units(f"{first} {second}")
    assert units[0] == first
    assert units[1] == " ".join(first.split()[-16:] + second.split())
    assert all(len(u.split()) <= UNIT_WORDS for u in units)


def test_a_sentence_longer_than_a_window_is_split_with_overlap():
    words = [f"w{i}" for i in range(250)]
    units = semantic_units(" ".join(words))
    assert [len(u.split()) for u in units] == [96, 96, 96, 34]
    assert units[1].split()[0] == "w72"
    assert {w for u in units for w in u.split()} == set(words)


def test_sentences_split_after_terminators_and_newlines():
    assert semantic_units("One here. Two there!\nThree?") == ["One here. Two there! Three?"]


def _word_chars(unit):
    return sum(len(w) for w in unit.split())


def test_a_long_unbroken_run_is_bounded_per_unit():
    # Mirrors `a_long_unbroken_run_is_bounded_per_unit`: 5000 chars become
    # 313 sixteen-char pieces, windowed 96 at a time with a 72-piece step.
    alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
    run = (alphabet * 100)[:5000]
    units = semantic_units(run)
    assert [len(u.split()) for u in units] == [96, 96, 96, 96, 25]
    assert all(_word_chars(u) <= UNIT_WORDS * 16 for u in units)
    assert all(len(w) <= 16 for u in units for w in u.split())
    assert run.startswith("".join(units[0].split()))


def test_full_width_terminators_end_sentences():
    assert semantic_units("甲乙。丙丁！戊己？庚辛；壬癸") == ["甲乙。 丙丁！ 戊己？ 庚辛； 壬癸"]


def test_a_cjk_run_without_punctuation_is_split_on_char_boundaries():
    run = ("数据保密协议" * 17)[:100]
    units = semantic_units(run)
    assert len(units) == 1
    pieces = units[0].split()
    assert len(pieces) == 7
    assert "".join(pieces) == run


def test_words_up_to_sixty_four_chars_are_kept_whole():
    word = "a" * 64
    assert semantic_units(f"see {word} here") == [f"see {word} here"]


def test_long_words_close_a_unit_before_it_outgrows_the_context():
    word = "b" * 60
    for text in (" ".join([word] * 120), " ".join([word + "."] * 120)):
        units = semantic_units(text)
        assert [len(u.split()) for u in units] == [25] * 6
        assert all(_word_chars(u) <= UNIT_WORDS * 16 for u in units)


def test_normalize_matches_the_rust_rule():
    assert normalize("  Public Filing,  TEXT! ") == "public filing text"
    assert normalize("--- ...") == ""


def test_family_split_alternates_over_sorted_names():
    fit = fitting_families(["c", "a", "b", "a", "d"])
    assert fit == {"a", "c"}


def test_family_split_halves_are_disjoint_and_cover_everything():
    names = [f"oida:{i:04d}" for i in range(41)]
    fit = fitting_families(names)
    held_out = set(names) - fit
    assert fit.isdisjoint(held_out)
    assert fit | held_out == set(names)
    assert len(fit) == 21 and len(held_out) == 20


def test_overlapping_texts_compares_after_normalization():
    anchors = ["Public filing text."]
    negatives = ["public  FILING text", "something else"]
    assert overlapping_texts(anchors, negatives) == {"public filing text"}


@pytest.mark.parametrize(
    "url",
    [
        "http://127.0.0.1:8765",
        "http://localhost:8080/v1",
        "http://[::1]:9000",
        "http://127.0.0.2:1234",
    ],
)
def test_loopback_urls_are_accepted(url):
    require_loopback(url)


@pytest.mark.parametrize(
    "url",
    [
        "http://10.0.0.5:8765",
        "https://api.example.com/v1",
        "http://localhost.example.com:8765",
        "http://0.0.0.0:8765",
        "http://192.168.1.10",
        "file:///tmp/socket",
        "http://127.0.0.1@evil.example.com/",
    ],
)
def test_non_loopback_urls_are_refused(url):
    with pytest.raises(ValueError):
        require_loopback(url)
