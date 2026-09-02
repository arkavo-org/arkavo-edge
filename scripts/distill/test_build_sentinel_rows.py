"""Pure functions and row construction for the sentinel row generator."""

from __future__ import annotations

from build_sentinel_rows import (
    REWRITE_EXCERPT_CHARS,
    accept_negative,
    accept_rewrite,
    build_row,
    build_tasks,
    generate,
    leaks,
)

POSITIVE = {
    "source_id": "doc001",
    "family": "oida:doc001",
    "sensitivity": "confidential",
    "category": "internal",
    "split": "train",
    "method": "verbatim",
    "page": 1,
    "n_pages": 1,
    "text": "the quick brown fox jumps over the lazy dog near the riverbank " * 3,
    "visual_tags": ["email address"],
    "task": "sentinel",
    "project": "unknown",
    "derived": ["doctype:email"],
    "source_url": "https://example.com/doc001",
}


def make_positive(source_id: str, split: str = "train") -> dict:
    row = dict(POSITIVE)
    row["source_id"] = source_id
    row["split"] = split
    row["family"] = f"oida:{source_id}"
    return row


# --- leaks ---------------------------------------------------------------


def test_leaks_true_when_12_word_run_present():
    source = "one two three four five six seven eight nine ten eleven twelve thirteen"
    candidate = "prefix text one two three four five six seven eight nine ten eleven twelve suffix"
    assert leaks(candidate, source) is True


def test_leaks_false_when_no_matching_run():
    source = "one two three four five six seven eight nine ten eleven twelve"
    candidate = "completely different words that share nothing with the source text at all here"
    assert leaks(candidate, source) is False


def test_leaks_false_when_source_too_short():
    assert leaks("one two three four five six seven eight nine ten eleven twelve", "short source text") is False


def test_leaks_true_when_12_word_window_straddles_newline():
    source = "one two three four five six\nseven eight nine ten eleven twelve"
    candidate = "prefix one two three four five six seven eight nine ten eleven twelve suffix"
    assert leaks(candidate, source) is True


def test_leaks_is_case_insensitive():
    source = "One Two Three Four Five Six Seven Eight Nine Ten Eleven Twelve"
    candidate = "one two three four five six seven eight nine ten eleven twelve"
    assert leaks(candidate, source) is True


# --- accept_negative -------------------------------------------------------


def test_accept_negative_requires_40_words():
    short_text = " ".join(["word"] * 39)
    long_text = " ".join(["word"] * 40)
    assert accept_negative(short_text) is False
    assert accept_negative(long_text) is True


def test_accept_negative_rejects_refusal_phrases():
    base = " ".join(["word"] * 45)
    assert accept_negative(f"I cannot help with that. {base}") is False
    assert accept_negative(f"I can't do that. {base}") is False
    assert accept_negative(f"As an AI, I won't. {base}") is False
    assert accept_negative(f"I'm sorry, no. {base}") is False
    assert accept_negative(base) is True


# --- accept_rewrite ---------------------------------------------------------


def test_accept_rewrite_requires_40pct_word_count():
    source = " ".join(["alpha"] * 100)
    too_short = " ".join(["beta"] * 39)
    long_enough = " ".join(f"beta{i}" for i in range(45))
    assert accept_rewrite(too_short, source) is False
    assert accept_rewrite(long_enough, source) is True


def test_accept_rewrite_rejects_near_identical_text():
    source = "alpha bravo charlie delta echo foxtrot golf hotel india juliet " * 5
    assert accept_rewrite(source, source) is False


def test_accept_rewrite_accepts_reworded_text_of_similar_length():
    source = "alpha bravo charlie delta echo foxtrot golf hotel india juliet " * 5
    reworded = "kilo lima mike november oscar papa quebec romeo sierra tango " * 5
    assert accept_rewrite(reworded, source) is True


# --- build_tasks / alternation ---------------------------------------------


def test_alternation_public_index0_internal_index1():
    train = [make_positive("t0"), make_positive("t1")]
    tasks = build_tasks(train, [])
    assert tasks[0].method == "synthetic-public"
    assert tasks[1].method == "synthetic-internal"


def test_eval_positive_produces_rewrite_and_alternating_negative():
    eval_rows = [make_positive("e0", split="eval"), make_positive("e1", split="eval")]
    tasks = build_tasks([], eval_rows)
    methods = [t.method for t in tasks]
    assert methods == ["rewrite", "synthetic-public", "rewrite", "synthetic-internal"]


# --- row construction --------------------------------------------------------


def test_build_row_public_negative_shape():
    pos = make_positive("t0")
    task = build_tasks([pos], [])[0]
    row = build_row(task, "generated public text", "test-generator")
    assert row["source_id"] == "t0-p1-pub"
    assert row["sensitivity"] == "public"
    assert row["category"] == "public"
    assert row["method"] == "synthetic-public"
    assert row["family"] == pos["family"]
    assert row["split"] == pos["split"]
    assert row["project"] == "unknown"
    assert row["derived"] == []
    assert row["visual_tags"] == []
    assert row["source_url"] == ""
    assert row["generator"] == "test-generator"


def test_build_row_internal_negative_shape():
    # index must be odd to get the internal-benign alternation
    pos = make_positive("t1")
    tasks = build_tasks([make_positive("t0"), pos], [])
    task = tasks[1]
    row = build_row(task, "generated internal text", "test-generator")
    assert row["source_id"] == "t1-p1-int"
    assert row["sensitivity"] == "internal"
    assert row["category"] == "internal"
    assert row["method"] == "synthetic-internal"
    assert row["family"] == pos["family"]
    assert row["split"] == pos["split"]


def test_build_row_rewrite_shape_keeps_positive_metadata():
    pos = make_positive("e0", split="eval")
    task = build_tasks([], [pos])[0]
    row = build_row(task, "rewritten text", "test-generator")
    assert row["source_id"] == "e0-p1-rw"
    assert row["sensitivity"] == "confidential"
    assert row["category"] == "internal"
    assert row["method"] == "rewrite"
    assert row["family"] == pos["family"]
    assert row["split"] == pos["split"]
    assert row["page"] == pos["page"]
    assert row["n_pages"] == pos["n_pages"]
    assert row["visual_tags"] == pos["visual_tags"]
    assert row["derived"] == pos["derived"]
    assert row["source_url"] == pos["source_url"]


# --- cache --------------------------------------------------------------------


def fake_chat_factory(word_count: int = 60):
    calls: list[tuple] = []

    def fake_chat(server, messages, max_tokens, seed=0, temperature=0.8, **_kw):
        calls.append((server, seed))
        return " ".join(f"generatedword{i}" for i in range(word_count))

    return fake_chat, calls


def test_cache_skips_regeneration_on_rerun(tmp_path):
    train = [make_positive("t0"), make_positive("t1")]
    tasks = build_tasks(train, [])

    fake_chat, calls = fake_chat_factory()
    rows, dropped = generate(tasks, tmp_path, fake_chat, "http://unused", 400, 2, "test-generator")
    assert len(calls) == 2
    assert len(rows) == 2
    assert sum(dropped.values()) == 0

    cache_file = tmp_path / "generations.jsonl"
    assert len(cache_file.read_text().splitlines()) == 2

    fake_chat2, calls2 = fake_chat_factory()
    rows2, dropped2 = generate(tasks, tmp_path, fake_chat2, "http://unused", 400, 2, "test-generator")
    assert len(calls2) == 0
    assert len(rows2) == 2
    assert len(cache_file.read_text().splitlines()) == 2


def test_cache_persists_rejected_rows_without_retry(tmp_path):
    train = [make_positive("t0")]
    tasks = build_tasks(train, [])

    def refusing_chat(server, messages, max_tokens, seed=0, temperature=0.8, **_kw):
        return "I'm sorry, I cannot write that."

    rows, dropped = generate(tasks, tmp_path, refusing_chat, "http://unused", 400, 2, "test-generator")
    assert rows == []
    assert dropped["synthetic-public"] == 1

    calls = []

    def counting_chat(server, messages, max_tokens, seed=0, temperature=0.8, **_kw):
        calls.append(1)
        return "should not be called"

    rows2, dropped2 = generate(tasks, tmp_path, counting_chat, "http://unused", 400, 2, "test-generator")
    assert calls == []
    assert rows2 == []
    assert dropped2["synthetic-public"] == 1


def test_rewrite_excerpt_constant_used_for_acceptance_source():
    # Sanity: the excerpt length constant used to build the rewrite prompt
    # is the same one acceptance compares against.
    assert REWRITE_EXCERPT_CHARS == 2500
