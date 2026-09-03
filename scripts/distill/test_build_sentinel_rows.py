"""Pure functions and row construction for the sentinel row generator."""

from __future__ import annotations

import json

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


# A realistic internal memo, >1000 chars, with the section-heading structure
# real corpus documents have. difflib.SequenceMatcher's autojunk heuristic
# (on by default for sequences >=200 elements) treats every common character
# as junk on text like this, which systematically inflates ratio() and lets
# a copy with two words swapped slip under the 0.9 rejection threshold --
# this is the exact failure mode I1 in the final review reports.
_MEMO_SOURCE = (
    "INTERNAL FACILITY MEMORANDUM\n"
    "Site: Hobart Manufacturing Plant\n"
    "Department: Quality Assurance\n"
    "Date: March 14, 2025\n"
    "Subject: Quarterly Compliance Review Summary\n\n"
    "Reference Number: QA-2025-0314\n"
    "Prepared By: R. Nikolaus\n"
    "Reviewed By: T. Alvarez\n"
    "Distribution: Site Leadership Team\n\n"
    "Summary of Findings:\n"
    "The quarterly compliance review identified no critical deviations. "
    "Batch record reconciliation for the extended-release tablet line was "
    "completed on schedule. Calibration records for analytical instruments "
    "in the quality control laboratory were verified against the master "
    "schedule maintained by the metrology group.\n\n"
    "Storage Conditions:\n"
    "Warehouse temperature logs for the reporting period showed readings "
    "within the validated range at all monitored points. Humidity sensors "
    "in Building 4 were recalibrated ahead of the scheduled interval.\n\n"
    "Training Status:\n"
    "All packaging line operators completed the annual refresher course. "
    "Training records were cross-checked against the roster maintained by "
    "Human Resources for the Hobart site.\n\n"
    "Change Control:\n"
    "Twelve proposed procedure changes were brought before the change "
    "control board. Nine were approved after minor revisions; three were "
    "returned for additional documentation.\n\n"
    "Next Steps:\n"
    "The site leadership team will review this summary at the next "
    "monthly quality meeting scheduled for the first week of April."
)


def test_accept_rewrite_rejects_near_verbatim_word_swap_in_long_source():
    assert len(_MEMO_SOURCE) > 1000
    words = _MEMO_SOURCE.split()
    # Swap two adjacent words ("board." and "Nine") deep in a sentence --
    # everything else is byte-for-byte identical to the source.
    i = words.index("board.")
    assert words[i + 1] == "Nine"
    near_verbatim = " ".join(words[:i] + [words[i + 1], words[i]] + words[i + 2 :])
    assert accept_rewrite(near_verbatim, _MEMO_SOURCE) is False


def test_accept_rewrite_accepts_genuine_reword_of_long_source():
    reworded = (
        "MEMO TO SITE STAFF\n"
        "Location: Hobart Plant\n"
        "Group: Quality Assurance\n"
        "Written: March 14, 2025\n"
        "Topic: Q1 Compliance Review Recap\n\n"
        "Tracking Number: QA-2025-0314\n"
        "Author: R. Nikolaus\n"
        "Approved By: T. Alvarez\n"
        "Sent To: Site Leadership Team\n\n"
        "What We Found:\n"
        "This quarter's compliance review turned up nothing serious. Records "
        "for the extended-release tablet line were matched against production "
        "logs and closed out on time. Every calibration entry for lab "
        "instruments used by quality control lined up with what the metrology "
        "group has on file.\n\n"
        "Storage:\n"
        "Warehouse temperatures stayed inside approved limits for the whole "
        "quarter, at every point we monitor. Humidity sensors in Building 4 "
        "got recalibrated early.\n\n"
        "Training:\n"
        "Everyone on the packaging line finished their yearly refresher. We "
        "cross-checked that against the HR roster for the Hobart site.\n\n"
        "Procedure Changes:\n"
        "The change board looked at twelve proposed edits. It approved nine "
        "after small tweaks; three went back for more paperwork.\n\n"
        "What's Next:\n"
        "Leadership will go over this recap at next month's quality meeting, "
        "set for the first week of April."
    )
    assert accept_rewrite(reworded, _MEMO_SOURCE) is True


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


def test_cache_hit_reevaluates_acceptance_with_current_rules(tmp_path):
    # A cache entry written under an older (or since-fixed) acceptance rule
    # can carry a stale accepted=True. A cache hit must re-run the current
    # accepted() predicate rather than trust that stored flag, so a fix to
    # the acceptance rules takes effect on rerun without regenerating text.
    train = [make_positive("t0")]
    tasks = build_tasks(train, [])
    cache_path = tmp_path / "generations.jsonl"
    tmp_path.mkdir(parents=True, exist_ok=True)
    stale = {
        "key": tasks[0].source_id,
        "method": tasks[0].method,
        "text": "I'm sorry, I cannot write that.",
        "accepted": True,  # stale: this text fails accept_negative today
    }
    cache_path.write_text(json.dumps(stale) + "\n")

    def must_not_be_called(*_a, **_kw):
        raise AssertionError("cache hit must not re-generate")

    rows, dropped = generate(tasks, tmp_path, must_not_be_called, "http://unused", 400, 2, "test-generator")
    assert rows == []
    assert dropped[tasks[0].method] == 1
    # The cache is left untouched: no regeneration happened.
    assert len(cache_path.read_text().splitlines()) == 1


# A source with genuinely different content before and after the 2500-char
# excerpt boundary. If acceptance ever compared against the full text
# instead of the REWRITE_EXCERPT_CHARS-char excerpt, a candidate that is
# exactly the excerpt would score well below the 0.9 threshold against the
# (longer, differently-worded) full text and be wrongly accepted.
_SOURCE_OVER_2500 = (
    "The quarterly compliance review at the Hobart manufacturing facility "
    "found no critical deviations across the sampled batch records. Quality "
    "assurance staff reconciled the extended-release tablet line records "
    "against the master production schedule and confirmed that every "
    "calibration entry for the analytical instruments in the quality "
    "control laboratory matched the metrology group's log. Warehouse "
    "temperature readings for the reporting period stayed within the "
    "validated range at every monitored point, and the humidity sensors in "
    "Building 4 were recalibrated ahead of their scheduled interval. Every "
    "operator assigned to the packaging line completed the required annual "
    "refresher course, and the training records were cross-checked against "
    "the roster maintained by Human Resources for the Hobart site. Of the "
    "twelve procedure changes brought before the change control board, nine "
    "were approved after minor revisions and three were returned for "
    "additional documentation before resubmission. No product recalls were "
    "initiated during the reporting period, and the volume of customer "
    "complaints tracked closely with the averages recorded for this product "
    "family over the preceding four quarters. The site leadership team will "
    "review this summary at the next monthly quality meeting, currently "
    "scheduled for the first full week of April, and will circulate the "
    "minutes to department heads no later than three business days "
    "afterward. Facilities management confirmed that the preventive "
    "maintenance schedule for the packaging line equipment remains on "
    "track, with no overdue work orders outstanding as of the review date. "
    "The environmental health and safety group also completed its monthly "
    "walkthrough of the packaging and warehouse areas without identifying "
    "any new corrective actions, and the fire suppression inspection "
    "certificates for both buildings remain current through the end of "
    "the calendar year. Procurement confirmed that the supplier "
    "qualification files for the two active raw-material vendors were "
    "renewed ahead of their expiration dates, and no new vendor deviations "
    "were logged during the period. The document control group closed out "
    "all pending revision requests for the site's standard operating "
    "procedures, bringing the master procedure index fully current as of "
    "the end of the quarter."
    " A second, unrelated section follows purely to push the source past "
    "the excerpt boundary with genuinely different material: the annual "
    "facility energy audit found that the packaging building's chiller "
    "plant was operating four percent below its rated efficiency, and "
    "engineering has scheduled a coil-cleaning service for next month. "
    "Separately, the loading dock's overhead door sensors were replaced "
    "after two intermittent fault codes were logged in the maintenance "
    "system, and the vendor confirmed the replacement parts carry a "
    "three-year warranty. Grounds crews completed the fall drainage "
    "inspection along the north perimeter without finding any blocked "
    "culverts, and the site's stormwater permit renewal paperwork was "
    "submitted to the state agency two weeks ahead of the deadline. IT "
    "operations migrated the badge-access system to its new server "
    "without any reported downtime, and helpdesk tickets related to badge "
    "readers dropped to zero in the week following the cutover. None of "
    "this later material appears anywhere in the first twenty-five "
    "hundred characters of the document."
)


def test_rewrite_acceptance_compares_against_the_2500_char_excerpt(tmp_path):
    assert len(_SOURCE_OVER_2500) > REWRITE_EXCERPT_CHARS
    pos = make_positive("e0", split="eval")
    pos["text"] = _SOURCE_OVER_2500
    tasks = build_tasks([], [pos])
    rewrite_task = tasks[0]
    assert rewrite_task.method == "rewrite"
    excerpt = _SOURCE_OVER_2500[:REWRITE_EXCERPT_CHARS]

    def fake_chat(server, messages, max_tokens, seed=0, temperature=0.8, **_kw):
        return excerpt  # exactly what the prompt excerpted, verbatim

    rows, dropped = generate([rewrite_task], tmp_path, fake_chat, "http://unused", 400, 1, "test-generator")
    assert rows == []
    assert dropped["rewrite"] == 1
