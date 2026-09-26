"""Tests for turning public source documents into anchor rows."""

import pytest

from pull_anchors import (
    MAX_ANCHOR_WORDS,
    bounded_sections,
    html_to_text,
    risk_factor_section,
    spl_sections,
    user_agent,
)


def test_html_to_text_drops_markup_scripts_and_entities():
    html = (
        "<html><head><style>p{}</style><script>var x=1;</script></head>"
        "<body><p>Opioid&nbsp;risk &amp; abuse.</p><div>Second<br>line</div></body></html>"
    )
    text = html_to_text(html)
    assert "var x" not in text and "p{}" not in text
    assert "Opioid risk & abuse." in text
    assert "Second" in text and "line" in text


def test_bounded_sections_never_exceed_the_word_cap():
    paragraphs = ["word " * 120 for _ in range(10)]
    sections = bounded_sections("\n\n".join(paragraphs))
    assert all(len(s.split()) <= MAX_ANCHOR_WORDS for s in sections)
    assert sum(len(s.split()) for s in sections) == 1200


def test_one_oversized_paragraph_is_split_at_word_boundaries():
    sections = bounded_sections("word " * (MAX_ANCHOR_WORDS * 2 + 10))
    assert [len(s.split()) for s in sections] == [MAX_ANCHOR_WORDS, MAX_ANCHOR_WORDS, 10]


def test_tiny_fragments_are_dropped():
    assert bounded_sections("Page 3 of 10") == []


def test_spl_sections_extracts_titled_text():
    xml = """<document xmlns="urn:hl7-org:v3"><component><structuredBody>
      <component><section><title>BOXED WARNING</title>
        <text><paragraph>Addiction, abuse and misuse.</paragraph></text></section></component>
      <component><section><title>INDICATIONS</title>
        <text><paragraph>For the management of pain.</paragraph></text></section></component>
    </structuredBody></component></document>"""
    sections = spl_sections(xml)
    assert sections == [
        "BOXED WARNING\nAddiction, abuse and misuse.",
        "INDICATIONS\nFor the management of pain.",
    ]


def test_risk_factor_section_is_cut_between_item_1a_and_the_next_item():
    text = (
        "Table of contents Item 1A. Risk Factors 23 Item 2. Properties 40\n"
        "Item 1. Business We make things.\n"
        "Item 1A. Risk Factors\nOpioid litigation could harm us.\n"
        "Item 1B. Unresolved Staff Comments\nNone."
    )
    assert risk_factor_section(text) == "Opioid litigation could harm us."


def test_a_mid_sentence_cross_reference_is_not_a_heading():
    text = (
        "Item 1A. Risk Factors\nOpioid litigation could harm us.\n"
        "Item 1B. Unresolved Staff Comments\nNone.\n"
        "Notes. Risks are described in Item 1A. Risk Factors of this report. "
        + "Inventory is measured at cost. " * 20
        + "\nItem 2. Properties\nPlants."
    )
    assert risk_factor_section(text) == "Opioid litigation could harm us."


def test_risk_factor_section_is_none_when_absent():
    assert risk_factor_section("Item 2. Properties only") is None


def test_the_user_agent_names_a_contact():
    assert user_agent("ops@example.org").endswith("(ops@example.org)")
    with pytest.raises(ValueError):
        user_agent("")
