"""Pull public, in-domain anchor documents for the Mallinckrodt pack.

Anchors are what the semantic tier subtracts: a prompt close to public
Mallinckrodt material (a drug label, a 10-K risk factor, a trial summary, a
regulator's press release) should not fire merely for being on-topic. They
must be real public text, never generated counterparts of protected
documents.

Sources, public endpoints only:
- DailyMed SPL labels for the Mallinckrodt and SpecGx labelers.
- SEC EDGAR 10-K and 10-Q filings of Mallinckrodt plc (Item 1A, Risk Factors).
- ClinicalTrials.gov v2 studies sponsored by Mallinckrodt.
- FDA and DEA press-release pages (drug-related FDA announcements; DEA's
  site refuses automated clients from some networks, which is recorded).

Every raw response is cached under `--cache`, so a rerun re-parses instead
of re-fetching. Rows are `{text, family, source}` with `family` the source
name. Long documents are cut into sections of at most `MAX_ANCHOR_WORDS`
words, and sections that mostly repeat an earlier one (successive labels
of the same product, year-over-year risk factors) are dropped: every anchor
is embedded into the sealed index, and a repeat costs size and scoring time
without adding anything to subtract.

Usage:
  python3 pull_anchors.py --cache DIR --out anchors.jsonl
"""

import argparse
import hashlib
import html
import json
import os
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request
import xml.etree.ElementTree as ET
from html.parser import HTMLParser

from common import normalize, write_jsonl

MAX_ANCHOR_WORDS = 400
MIN_ANCHOR_WORDS = 8
REPEAT_FRACTION = 0.5
MALLINCKRODT_CIK = "0001567892"
DAILYMED_LABELERS = ("Mallinckrodt", "SpecGx")
FDA_PAGES = 50
DRUG_TERMS = re.compile(
    r"opioid|drug|pharmac|fentanyl|naloxone|oxycodone|hydrocodone|controlled substance|"
    r"prescription|analgesic|addiction|overdose|pain",
    re.IGNORECASE,
)

_SPL = "{urn:hl7-org:v3}"
_BLOCK = {"p", "div", "br", "li", "tr", "h1", "h2", "h3", "h4", "h5", "h6", "table", "section"}
# Headings only: a filing cross-references "Item 1A. Risk Factors" mid-sentence
# many times, and a span cut from one of those runs into the financial notes.
_ITEM_1A = re.compile(r"^[ \t]*item\s*1a\.?\s*[\-—:]?\s*risk\s+factors", re.I | re.M)
_NEXT_ITEM = re.compile(r"^[ \t]*item\s*(?:1b|2)\s*\.", re.I | re.M)


def user_agent(email):
    if not email:
        raise ValueError("a contact email is required for a polite User-Agent")
    return f"arkavo-edge semantic-tier anchor pull ({email})"


class _Text(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.parts = []
        self._skip = 0

    def handle_starttag(self, tag, attrs):
        if tag in ("script", "style"):
            self._skip += 1
        elif tag in _BLOCK:
            self.parts.append("\n")

    def handle_endtag(self, tag):
        if tag in ("script", "style"):
            self._skip = max(0, self._skip - 1)
        elif tag in _BLOCK:
            self.parts.append("\n")

    def handle_data(self, data):
        if not self._skip:
            self.parts.append(data)


def html_to_text(markup):
    parser = _Text()
    parser.feed(markup)
    parser.close()
    text = html.unescape("".join(parser.parts)).replace("\xa0", " ")
    lines = [re.sub(r"[ \t\r\f\v]+", " ", line).strip() for line in text.split("\n")]
    return re.sub(r"\n{3,}", "\n\n", "\n".join(lines)).strip()


def bounded_sections(text, max_words=MAX_ANCHOR_WORDS):
    """Pack paragraphs into sections of at most `max_words` words."""
    sections, current = [], []
    for paragraph in re.split(r"\n\s*\n", text):
        words = paragraph.split()
        while len(words) > max_words:
            if current:
                sections.append(current)
                current = []
            sections.append(words[:max_words])
            words = words[max_words:]
        if current and len(current) + len(words) > max_words:
            sections.append(current)
            current = []
        current.extend(words)
    if current:
        sections.append(current)
    return [" ".join(s) for s in sections if len(s) >= MIN_ANCHOR_WORDS]


def spl_sections(xml_text):
    """Top-level SPL sections as `title\\nbody`, nested subsections included."""
    # stdlib expat on a supported Python neither resolves external entities
    # nor expands entity bombs, which keeps this script dependency-free.
    root = ET.fromstring(xml_text)
    out = []
    for body in root.iter(f"{_SPL}structuredBody"):
        for component in body.findall(f"{_SPL}component"):
            section = component.find(f"{_SPL}section")
            if section is None:
                continue
            title_el = section.find(f"{_SPL}title")
            title = " ".join("".join(title_el.itertext()).split()) if title_el is not None else ""
            text = " ".join(
                " ".join(child.itertext()) for child in section if child is not title_el
            )
            text = " ".join(text.split())
            if text:
                out.append(f"{title}\n{text}" if title else text)
    return out


def risk_factor_section(text):
    """Item 1A of a 10-K/10-Q: the longest span from an "Item 1A. Risk Factors"
    heading to the next Item 1B/2 heading (a table of contents yields a short
    one). A heading with no following one is not a section boundary."""
    best = None
    for match in _ITEM_1A.finditer(text):
        end = _NEXT_ITEM.search(text, match.end())
        if end is None:
            continue
        span = text[match.end() : end.start()].strip()
        if best is None or len(span) > len(best):
            best = span
    return best


class Fetcher:
    """Polite, cached HTTP GET: one request at a time, a pause between them."""

    def __init__(self, cache_dir, agent, pause):
        self.cache_dir = cache_dir
        self.agent = agent
        self.pause = pause
        self.failures = {}
        os.makedirs(cache_dir, exist_ok=True)

    def get(self, source, url):
        name = hashlib.sha256(url.encode()).hexdigest()[:24]
        path = os.path.join(self.cache_dir, source, name)
        if os.path.exists(path):
            with open(path, encoding="utf-8") as handle:
                return handle.read()
        request = urllib.request.Request(url, headers={"User-Agent": self.agent})
        try:
            with urllib.request.urlopen(request, timeout=60) as response:
                body = response.read().decode("utf-8", errors="replace")
        except (urllib.error.URLError, TimeoutError) as err:
            self.failures.setdefault(source, []).append(f"{url}: {err}")
            return None
        finally:
            time.sleep(self.pause)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w", encoding="utf-8") as handle:
            handle.write(body)
        return body


def dailymed(fetch):
    rows = []
    for labeler in DAILYMED_LABELERS:
        page = 1
        while True:
            url = (
                "https://dailymed.nlm.nih.gov/dailymed/services/v2/spls.json"
                f"?labeler={labeler}&pagesize=100&page={page}"
            )
            listing = fetch.get("dailymed", url)
            if listing is None:
                break
            data = json.loads(listing)
            for spl in data["data"]:
                setid = spl["setid"]
                xml_text = fetch.get(
                    "dailymed", f"https://dailymed.nlm.nih.gov/dailymed/services/v2/spls/{setid}.xml"
                )
                if xml_text is None:
                    continue
                for section in spl_sections(xml_text):
                    rows.extend(_rows(section, "dailymed", f"dailymed:{setid}"))
            if page >= int(data["metadata"]["total_pages"]):
                break
            page += 1
    return rows


def edgar(fetch):
    listing = fetch.get("edgar", f"https://data.sec.gov/submissions/CIK{MALLINCKRODT_CIK}.json")
    if listing is None:
        return []
    recent = json.loads(listing)["filings"]["recent"]
    rows = []
    cik = str(int(MALLINCKRODT_CIK))
    for form, accession, document in zip(
        recent["form"], recent["accessionNumber"], recent["primaryDocument"]
    ):
        if form not in ("10-K", "10-Q"):
            continue
        url = f"https://www.sec.gov/Archives/edgar/data/{cik}/{accession.replace('-', '')}/{document}"
        body = fetch.get("edgar", url)
        section = risk_factor_section(html_to_text(body)) if body else None
        if section:
            rows.extend(_rows(section, "sec-edgar", f"sec-edgar:{accession}:{form}"))
    return rows


def clinicaltrials(fetch):
    rows = []
    token = ""
    while True:
        url = (
            "https://clinicaltrials.gov/api/v2/studies?query.spons=Mallinckrodt&pageSize=100"
            + (f"&pageToken={token}" if token else "")
        )
        listing = fetch.get("clinicaltrials", url)
        if listing is None:
            break
        data = json.loads(listing)
        for study in data.get("studies", []):
            protocol = study.get("protocolSection", {})
            ident = protocol.get("identificationModule", {})
            desc = protocol.get("descriptionModule", {})
            parts = [
                ident.get("officialTitle") or ident.get("briefTitle", ""),
                desc.get("briefSummary", ""),
                desc.get("detailedDescription", ""),
            ]
            text = "\n\n".join(p for p in parts if p)
            rows.extend(_rows(text, "clinicaltrials", f"clinicaltrials:{ident.get('nctId')}"))
        token = data.get("nextPageToken")
        if not token:
            break
    return rows


def fda_press(fetch):
    rows = []
    for page in range(FDA_PAGES):
        listing = fetch.get(
            "fda", f"https://www.fda.gov/news-events/fda-newsroom/press-announcements?page={page}"
        )
        if listing is None:
            break
        links = dict.fromkeys(re.findall(r'href="(/news-events/press-announcements/[^"#?]+)"', listing))
        for link in links:
            body = fetch.get("fda", f"https://www.fda.gov{link}")
            if body is None:
                continue
            article = re.search(r'<article[^>]*id="main-content".*?</article>', body, re.DOTALL)
            text = html_to_text(article.group(0)) if article else ""
            if DRUG_TERMS.search(text):
                rows.extend(_rows(text, "fda-press", f"fda-press:{link.rsplit('/', 1)[-1]}"))
    return rows


def dea_press(fetch):
    listing = fetch.get("dea", "https://www.dea.gov/what-we-do/news/press-releases")
    if listing is None:
        return []
    rows = []
    for link in dict.fromkeys(re.findall(r'href="(/press-releases/[^"#?]+)"', listing)):
        body = fetch.get("dea", f"https://www.dea.gov{link}")
        if body:
            rows.extend(_rows(html_to_text(body), "dea-press", f"dea-press:{link}"))
    return rows


def _rows(text, family, source):
    return [{"text": s, "family": family, "source": source} for s in bounded_sections(text)]


def drop_repeats(rows, fraction=REPEAT_FRACTION):
    """Keep a row unless `fraction` of its five-word shingles already appear
    in rows kept before it."""
    seen = set()
    kept = []
    for row in rows:
        words = normalize(row["text"]).split()
        shingles = {" ".join(words[i : i + 5]) for i in range(max(1, len(words) - 4))}
        if shingles and len(shingles & seen) / len(shingles) >= fraction:
            continue
        seen |= shingles
        kept.append(row)
    return kept


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--cache", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--pause", type=float, default=0.3)
    args = parser.parse_args(argv)

    email = subprocess.run(
        ["git", "config", "user.email"], capture_output=True, text=True, check=False
    ).stdout.strip()
    fetch = Fetcher(args.cache, user_agent(email), args.pause)

    rows = []
    for name, pull in (
        ("dailymed", dailymed),
        ("sec-edgar", edgar),
        ("clinicaltrials", clinicaltrials),
        ("fda-press", fda_press),
        ("dea-press", dea_press),
    ):
        pulled = pull(fetch)
        documents = len({r["source"] for r in pulled})
        print(f"{name}: {documents} documents, {len(pulled)} sections", flush=True)
        rows.extend(pulled)

    kept = drop_repeats(rows)
    write_jsonl(args.out, kept)
    by_family = {}
    for row in kept:
        by_family.setdefault(row["family"], set()).add(row["source"])
    for family, sources in sorted(by_family.items()):
        sections = sum(1 for r in kept if r["family"] == family)
        print(f"kept {family}: {len(sources)} documents, {sections} sections")
    print(f"kept {len({r['source'] for r in kept})} documents, {len(kept)} anchor rows "
          f"({len(rows) - len(kept)} repeating sections dropped)")
    for source, errors in fetch.failures.items():
        print(f"FAILED {source}: {len(errors)} requests, first: {errors[0]}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
