#!/usr/bin/env python3
"""Northwind example corpus for the Qwen3.5-0.8B sentinel distill.

Split is by source document, never by paraphrase. Train derivations are
slot-filled restatements. Eval derivations are handwritten rewrites so a
number on eval is not 'the generator recognised itself'.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

# Fictional demo company. No real people, no real account numbers.
# Credential-shaped strings are generated at eval time, not stored here.

SOURCES: list[dict] = [
    {
        "id": "board-acq-timing",
        "family": "northwind-board",
        "sensitivity": "confidential",
        "category": "internal",
        "text": (
            "The Northwind acquisition closes in the third quarter pending board approval. "
            "Directors will vote on 14 September in a closed session. Public announcement "
            "is forbidden until the vote is recorded."
        ),
        "slots": [
            "Northwind's purchase wraps up in Q3 if the board says yes. The closed vote is 14 September. Nobody speaks publicly before that.",
            "Pending director approval, the Northwind deal completes this third quarter. The 14 September session is closed. Keep it off the wire.",
        ],
        "rewrite": (
            "If the directors agree in their private September meeting, Northwind will be "
            "bought before October. Until they record that vote, the story stays inside the room."
        ),
    },
    {
        "id": "board-indemnity",
        "family": "northwind-board",
        "sensitivity": "confidential",
        "category": "financial",
        "text": (
            "The indemnity cap on the Northwind share purchase is forty million, with a "
            "twelve-month survival on general reps. Special indemnity for the warehouse "
            "lease survives twenty-four months."
        ),
        "slots": [
            "Northwind SPA indemnity is capped at 40 million. Ordinary reps last a year; the warehouse lease indemnity lasts two.",
            "Share purchase indemnity for Northwind: 40m ceiling, 12-month general survival, 24-month warehouse-lease survival.",
        ],
        "rewrite": (
            "Sellers of Northwind cover losses only up to forty million, and only for a year "
            "on the usual warranties. The warehouse lease is the exception: that cover lasts two years."
        ),
    },
    {
        "id": "board-valuation",
        "family": "northwind-board",
        "sensitivity": "confidential",
        "category": "financial",
        "text": (
            "Management's internal valuation of Northwind is 180 million on a cash-free "
            "debt-free basis. The banker range presented to the board was 165 to 195 million. "
            "That range is not to leave this pack."
        ),
        "slots": [
            "Inside number for Northwind: 180m CFDF. Bankers showed the board 165–195. Do not circulate the range.",
            "Northwind is carried internally at one hundred eighty million cash-free debt-free. Board book range 165 to 195 million stays in the pack.",
        ],
        "rewrite": (
            "We are treating Northwind as worth one hundred eighty million without cash or debt. "
            "Advisers put a band around that for the directors. The band is pack-only."
        ),
    },
    {
        "id": "hr-comp-bands",
        "family": "northwind-hr",
        "sensitivity": "confidential",
        "category": "internal",
        "text": (
            "Proposed Northwind retention: the three named leads stay on their current cash "
            "plus a one-time stay bonus equal to four months. Band for those roles is held "
            "in the HR pack, not in the all-hands slides."
        ),
        "slots": [
            "Keep the three Northwind leads. Same cash, stay bonus of four months. Those bands live in HR, not in the town hall deck.",
            "Northwind retention plan: three leads, four-month stay bonus, cash unchanged. Compensation figures stay out of all-hands.",
        ],
        "rewrite": (
            "We want the three Northwind leads to remain. Pay does not change; they get a "
            "four-month stay payment. Do not put those numbers on a slide the whole company sees."
        ),
    },
    {
        "id": "hr-reorg",
        "family": "northwind-hr",
        "sensitivity": "confidential",
        "category": "internal",
        "text": (
            "If the vote carries, Northwind's warehouse team consolidates into Cedar's ops "
            "in November. Fourteen roles overlap. Names and selection criteria sit with HR; "
            "this note only records the count and the month."
        ),
        "slots": [
            "Post-vote, warehouse staff fold into Cedar ops in November. Fourteen overlapping seats. HR holds the names.",
            "November consolidation of Northwind warehouse into Cedar operations. Overlap: 14 roles. Identities are not in this note.",
        ],
        "rewrite": (
            "After a yes vote the Northwind warehouse people join Cedar operations in November. "
            "Fourteen jobs cover the same work. Who is selected is an HR file, not this one."
        ),
    },
    {
        "id": "eng-sprint",
        "family": "northwind-eng",
        "sensitivity": "internal",
        "category": "internal",
        "text": (
            "Sprint 18 review is Wednesday at three in the big room. Bring the demo laptop. "
            "On-call this week is the payments pair. No customer names in the notes."
        ),
        "slots": [
            "Wednesday 3pm, big room: sprint 18. Demo laptop required. Payments pair is on-call. Leave customers out of the write-up.",
            "Sprint 18 ships a review Wednesday afternoon in the large room. Bring the demo machine. Payments owns on-call.",
        ],
        "rewrite": (
            "We look at sprint 18 on Wednesday afternoon in the large conference room. "
            "Whoever demos should carry the laptop. Payments engineers are on-call. The notes name no customers."
        ),
    },
    {
        "id": "eng-runbook",
        "family": "northwind-eng",
        "sensitivity": "internal",
        "category": "internal",
        "text": (
            "The billing worker retries twice then parks the job on the dead-letter queue. "
            "Page the payments pair if the queue is above fifty. This is the internal runbook, "
            "not a customer promise."
        ),
        "slots": [
            "Billing worker: two retries, then dead-letter. Page payments when dead-letter exceeds 50. Internal only.",
            "Dead-letter the billing job after two retries. Alert payments above fifty parked jobs. Not an SLA.",
        ],
        "rewrite": (
            "If billing fails, try twice and then stop; the job waits on the dead-letter queue. "
            "Wake payments when that queue passes fifty. Customers are not told this number."
        ),
    },
    {
        "id": "public-launch",
        "family": "public-blog",
        "sensitivity": "public",
        "category": "public",
        "text": (
            "Today we opened a small clinic on Harbor Street. Walk-ins welcome on weekdays. "
            "The ribbon cutting is on the town calendar."
        ),
        "slots": [
            "Harbor Street clinic is open. Weekday walk-ins. Ribbon cutting is a public calendar event.",
            "We launched the Harbor Street clinic for weekday walk-ins. The town listed the opening.",
        ],
        "rewrite": (
            "A new weekday walk-in clinic is operating on Harbor Street. The opening was posted for anyone to attend."
        ),
    },
    {
        "id": "public-hiring",
        "family": "public-blog",
        "sensitivity": "public",
        "category": "public",
        "text": (
            "We are hiring a clinic nurse and a front-desk coordinator. Apply on the careers "
            "page. Pay is listed there. This post is for the public site."
        ),
        "slots": [
            "Public careers page: clinic nurse and front-desk coordinator. Compensation is on that page.",
            "Two public openings: nurse and coordinator. Send people to the careers listing.",
        ],
        "rewrite": (
            "The public careers site has two roles — a nurse and a coordinator — including what they pay."
        ),
    },
    {
        "id": "public-talk",
        "family": "public-blog",
        "sensitivity": "public",
        "category": "public",
        "text": (
            "On Friday the director gives a town-library talk on keeping paper records dry. "
            "No patient stories. The slides are the ones already on the library site."
        ),
        "slots": [
            "Friday library talk: dry paper records. Public slides. No patient anecdotes.",
            "Director speaks at the library Friday about paper storage. Materials are already public.",
        ],
        "rewrite": (
            "The director's Friday library lecture is about storing paper so it does not rot. "
            "The slides are the public set. Patients are not discussed."
        ),
    },
    {
        "id": "public-science",
        "family": "public-blog",
        "sensitivity": "public",
        "category": "public",
        "text": (
            "Photosynthesis converts light energy into chemical energy in the chloroplasts "
            "of green plants. This paragraph is a textbook sentence, not company material."
        ),
        "slots": [
            "Green plants store sunlight as chemical energy in chloroplasts. Public textbook fact.",
            "Chloroplasts run photosynthesis. That is general science, not an internal note.",
        ],
        "rewrite": (
            "Leaves turn sunlight into stored energy. Anyone's biology book says so."
        ),
    },
    {
        "id": "public-weather",
        "family": "public-blog",
        "sensitivity": "public",
        "category": "public",
        "text": (
            "The harbor forecast is rain after noon. Boats should be tied. This is the "
            "same notice pinned on the public board."
        ),
        "slots": [
            "Public board: rain after noon, tie the boats.",
            "Harbor weather is public. Afternoon rain. Secure the dinghies.",
        ],
        "rewrite": (
            "Rain is expected on the harbor this afternoon. The notice is on the town board."
        ),
    },
    {
        "id": "eng-readme",
        "family": "northwind-eng",
        "sensitivity": "internal",
        "category": "internal",
        "text": (
            "The example agent listens on loopback port 8787. Copy .env.example to .env. "
            "This is the internal getting-started page, not a production hostname."
        ),
        "slots": [
            "Internal readme: loopback 8787, start from .env.example. Not a public host.",
            "Dev listen address is 127.0.0.1:8787. Documented for the team, not for customers.",
        ],
        "rewrite": (
            "Developers run the sample on localhost port 8787 after copying the example env file. "
            "That page is for the team."
        ),
    },
]


# Entire sources held out of train. Eval on these is "unseen document",
# not paraphrase of something the trainer already hashed.
HOLDOUT_SOURCES = frozenset({"board-valuation", "public-talk"})


def records() -> list[dict]:
    out: list[dict] = []
    for src in SOURCES:
        base = {
            "source_id": src["id"],
            "family": src["family"],
            "sensitivity": src["sensitivity"],
            "category": src["category"],
        }
        hold = src["id"] in HOLDOUT_SOURCES
        if hold:
            out.append({**base, "split": "eval", "method": "unseen-verbatim", "text": src["text"]})
            out.append({**base, "split": "eval", "method": "unseen-rewrite", "text": src["rewrite"]})
            continue
        out.append({**base, "split": "train", "method": "verbatim", "text": src["text"]})
        for i, slot in enumerate(src["slots"]):
            out.append({**base, "split": "train", "method": f"slot-{i}", "text": slot})
        out.append({**base, "split": "eval", "method": "rewrite", "text": src["rewrite"]})
    return out


def leakage(rows: list[dict]) -> list[str]:
    """Holdout sources must not appear in train; train sources must not be eval-unseen."""
    train_ids = {r["source_id"] for r in rows if r["split"] == "train"}
    bad: list[str] = []
    for src_id in HOLDOUT_SOURCES:
        if src_id in train_ids:
            bad.append(src_id)
    for row in rows:
        if row["split"] == "eval" and row["method"].startswith("unseen") and row["source_id"] in train_ids:
            bad.append(row["source_id"])
    return bad


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    rows = records()
    leaked = leakage(rows)
    if leaked:
        raise SystemExit(f"eval contains verbatim sources: {leaked}")
    args.out.mkdir(parents=True, exist_ok=True)
    train = [r for r in rows if r["split"] == "train"]
    eval_rows = [r for r in rows if r["split"] == "eval"]
    (args.out / "train.json").write_text(json.dumps(train, indent=2) + "\n")
    (args.out / "eval.json").write_text(json.dumps(eval_rows, indent=2) + "\n")
    (args.out / "sources.json").write_text(json.dumps(SOURCES, indent=2) + "\n")
    print(
        f"wrote {len(train)} train and {len(eval_rows)} eval rows "
        f"from {len(SOURCES)} sources to {args.out}"
    )


if __name__ == "__main__":
    main()
