"""Topic names the benign negatives are generated from.

Each topic becomes one calibration family, so the build's alternating split
holds out whole topics: the false-positive rate is measured on subjects the
threshold was never fitted to. The in-domain topics are written from general
public knowledge of the industry and name no document in the corpus.
"""

GENERAL_TOPICS = (
    "cooking weeknight dinners",
    "planning a vacation abroad",
    "learning to program in Python",
    "personal budgeting and saving",
    "home gardening",
    "fitness and strength training",
    "writing a cover letter",
    "car maintenance",
    "raising young children",
    "world history",
    "basic algebra and statistics",
    "choosing a laptop",
    "learning a foreign language",
    "pet care for dogs and cats",
    "home repair and DIY",
    "movies and television recommendations",
    "sleep and stress management",
    "renting or buying a home",
    "writing emails at work",
    "astronomy and space exploration",
    "board games and puzzles",
    "climate and weather",
    "small business marketing",
    "music theory and playing guitar",
    "spreadsheet formulas",
)

DOMAIN_TOPICS = (
    "how opioid analgesics work in the body",
    "DEA controlled substance schedules",
    "DEA aggregate production quotas",
    "FDA opioid REMS programs",
    "opioid prescribing guidelines for chronic pain",
    "naloxone and overdose reversal",
    "medication for opioid use disorder",
    "generic drug manufacturing and ANDA approval",
    "pharmaceutical wholesale distribution",
    "suspicious order monitoring requirements for distributors",
    "pharmacy dispensing rules for controlled substances",
    "prescription drug monitoring programs",
    "the history of the US opioid epidemic",
    "opioid litigation and national settlements",
    "pharmaceutical company bankruptcy and restructuring",
    "drug pricing and rebates",
    "pharmaceutical sales representatives and marketing rules",
    "FDA drug labeling and boxed warnings",
    "clinical trials for pain medications",
    "abuse-deterrent opioid formulations",
    "pharmaceutical supply chain and chargebacks",
    "SEC disclosures by pharmaceutical companies",
    "hospital pharmacy formulary decisions",
    "acetaminophen and combination pain products",
    "public health data on overdose deaths",
)
