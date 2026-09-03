#!/usr/bin/env python3
"""Topic-matched public negatives for the OIDA sentinel.

Without these the classifier learns "pharma" instead of "internal versus
public". Texts are public-record material about the same companies: FDA
prescribing information, SEC-style risk-factor language, trial registry
entries, and press-release facts. They are labeled public and carry no
project decrypt attribute — public release is the point.
"""

from __future__ import annotations

import json
from pathlib import Path

# US-government and exchange-filed language about the same firms the archive
# covers. Not a substitute for a live EDGAR/DailyMed pull; enough that a
# sentinel trained on OIDA internals has to see the public counterpart.

NEGATIVES: list[dict] = [
    {
        "source_id": "fda-oxycodone-label",
        "family": "public-fda",
        "company": "mallinckrodt",
        "kind": "fda-label",
        "text": (
            "HIGHLIGHTS OF PRESCRIBING INFORMATION. These highlights do not include "
            "all the information needed to use oxycodone hydrochloride tablets safely "
            "and effectively. See full prescribing information. WARNING: SERIOUS AND "
            "LIFE-THREATENING RISKS FROM USE OF OXYCODONE HYDROCHLORIDE. Addiction, "
            "abuse, and misuse: oxycodone hydrochloride exposes patients and other "
            "users to the risks of opioid addiction, abuse, and misuse, which can "
            "lead to overdose and death. Assess each patient's risk prior to "
            "prescribing and reassess all patients regularly. Life-threatening "
            "respiratory depression: serious, life-threatening, or fatal respiratory "
            "depression may occur. Monitor for respiratory depression, especially "
            "during initiation or following a dose increase."
        ),
    },
    {
        "source_id": "fda-actiq-label",
        "family": "public-fda",
        "company": "teva",
        "kind": "fda-label",
        "text": (
            "ACTIQ (fentanyl citrate) oral transmucosal lozenge is indicated for the "
            "management of breakthrough pain in cancer patients 16 years of age and "
            "older who are already receiving and who are tolerant to around-the-clock "
            "opioid therapy for their underlying persistent cancer pain. Patients "
            "considered opioid tolerant are those who are taking, for one week or "
            "longer, at least 60 mg oral morphine per day. ACTIQ is available only "
            "through a restricted program called TIRF REMS because of the risk of "
            "misuse, abuse, addiction, and overdose."
        ),
    },
    {
        "source_id": "sec-mallinckrodt-opioid-risk",
        "family": "public-sec",
        "company": "mallinckrodt",
        "kind": "10-k",
        "text": (
            "We manufacture and distribute opioid products and are subject to "
            "extensive regulation by the FDA and the DEA. We have been named in "
            "litigation relating to the marketing and distribution of opioid "
            "medications. The outcome of such litigation and related governmental "
            "investigations could have a material adverse effect on our financial "
            "condition. These statements are contained in reports filed with the "
            "U.S. Securities and Exchange Commission and are available to the public."
        ),
    },
    {
        "source_id": "sec-endo-opioid-risk",
        "family": "public-sec",
        "company": "endo",
        "kind": "10-k",
        "text": (
            "Endo International plc has disclosed in filings with the Securities and "
            "Exchange Commission that it faces substantial litigation and regulatory "
            "scrutiny related to its opioid medicines, including Opana ER. The company "
            "has described potential liability, governmental investigations, and the "
            "possibility of bankruptcy as risk factors. Those disclosures are public "
            "SEC filings, not internal sales correspondence."
        ),
    },
    {
        "source_id": "sec-teva-opioid-risk",
        "family": "public-sec",
        "company": "teva",
        "kind": "10-k",
        "text": (
            "Teva Pharmaceutical Industries Ltd. has reported in public SEC filings "
            "that it manufactures generic and branded opioid products, including "
            "products acquired with Cephalon, and that it is a defendant in opioid "
            "litigation in the United States. The company has described settlement "
            "agreements and injunctive terms as material events. This is a public "
            "risk-factor disclosure."
        ),
    },
    {
        "source_id": "nct-opioid-trial",
        "family": "public-trials",
        "company": "endo",
        "kind": "clinicaltrials",
        "text": (
            "A randomized, double-blind, placebo-controlled study of an extended-release "
            "oxymorphone formulation for chronic low back pain is registered on "
            "ClinicalTrials.gov. Inclusion criteria, primary endpoints, and sponsor "
            "identity are public. Enrollment numbers and outcome measures appear on "
            "the registry record and in resulting publications."
        ),
    },
    {
        "source_id": "mallinckrodt-press-bankruptcy",
        "family": "public-press",
        "company": "mallinckrodt",
        "kind": "press-release",
        "text": (
            "Mallinckrodt plc announced publicly that it had filed voluntary petitions "
            "under Chapter 11 and that a court-sanctioned agreement would resolve "
            "opioid-related claims. The press release, the docket, and subsequent "
            "reorganization plans are public court and investor communications, not "
            "internal compensation or targeting files."
        ),
    },
    {
        "source_id": "mckinsey-press-settlement",
        "family": "public-press",
        "company": "mckinsey",
        "kind": "press-release",
        "text": (
            "McKinsey & Company announced a settlement with U.S. state attorneys "
            "general relating to its past consulting work for opioid manufacturers. "
            "The settlement amount, the injunctive terms, and the requirement to "
            "produce documents to a public archive were described in public "
            "statements by the firm and by the states."
        ),
    },
]


def records() -> list[dict]:
    out: list[dict] = []
    for src in NEGATIVES:
        out.append(
            {
                "source_id": src["source_id"],
                "family": src["family"],
                "sensitivity": "public",
                "category": "public",
                "split": "train",
                "method": "verbatim",
                "text": src["text"],
                "task": "sentinel",
                "kind": src["kind"],
                "company": src["company"],
            }
        )
    return out


def main() -> None:
    import argparse

    parser = argparse.ArgumentParser()
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    rows = records()
    (args.out / "public.jsonl").write_text(
        "".join(json.dumps(r, ensure_ascii=False) + "\n" for r in rows)
    )
    print(f"wrote {len(rows)} public negative rows to {args.out / 'public.jsonl'}")


if __name__ == "__main__":
    main()
