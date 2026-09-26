"""Helpers shared by the semantic-tier measurement scripts.

The chunker, the normalisation rule and the family split are ports of
`arkavo-fingerprint` (`embed.rs`, `shingle.rs`, `semantic_calibration.rs`).
They are ported rather than approximated because the scripts must pick the
same units the index stores, and must know in advance which families the
build will hold out; a near-miss port would make "held-out" mean something
the build never did.
"""

import ipaddress
import json
import re
from urllib.parse import urlsplit

UNIT_WORDS = 96
UNIT_OVERLAP = 24

_SENTENCE = re.compile(r"[^.!?\n]*[.!?\n]|[^.!?\n]+$")


def read_jsonl(path):
    with open(path, encoding="utf-8") as handle:
        return [json.loads(line) for line in handle if line.strip()]


def write_jsonl(path, rows):
    with open(path, "w", encoding="utf-8") as handle:
        for row in rows:
            handle.write(json.dumps(row, ensure_ascii=False) + "\n")


def _sentences(text):
    # Rust's `split_inclusive(['.', '!', '?', '\n'])`: every piece keeps its
    # terminator, and a trailing piece without one is kept too.
    return [m.group(0) for m in _SENTENCE.finditer(text) if m.group(0)]


def semantic_units(text):
    """Split text into the units the index stores (port of `semantic_units`)."""
    units = []
    window = []
    for sentence in _sentences(text):
        words = sentence.split()
        if not words:
            continue
        if len(words) > UNIT_WORDS:
            _flush(units, window)
            _split_long(words, units)
            continue
        if len(window) + len(words) > UNIT_WORDS:
            room = UNIT_WORDS - len(words)
            keep = min(UNIT_OVERLAP, room)
            carried = window[len(window) - keep :] if keep else []
            _flush(units, window)
            window.extend(carried)
        window.extend(words)
    _flush(units, window)
    return units


def _flush(units, window):
    if window:
        units.append(" ".join(window))
        window.clear()


def _split_long(words, units):
    step = UNIT_WORDS - UNIT_OVERLAP
    start = 0
    while True:
        end = min(start + UNIT_WORDS, len(words))
        units.append(" ".join(words[start:end]))
        if end == len(words):
            return
        start += step


def normalize(text):
    """Port of `arkavo_fingerprint::normalize`: lowercase, ASCII punctuation
    trimmed from each word, whitespace collapsed."""
    out = []
    for token in text.split():
        trimmed = token.strip("!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~")
        if trimmed:
            out.append(trimmed.lower())
    return " ".join(out)


def fitting_families(families):
    """Families the build fits a threshold on: sorted distinct names at even
    positions. Everything else is held out and measured."""
    return {name for i, name in enumerate(sorted(set(families))) if i % 2 == 0}


def overlapping_texts(anchors, negatives):
    """Normalised texts present in both sets; the build refuses any."""
    anchor_norms = {normalize(a) for a in anchors}
    return {normalize(n) for n in negatives} & anchor_norms


def require_loopback(url):
    """Refuse any server that is not on this machine.

    Confidential chunks are sent to this server; a remote endpoint would be
    exactly the leak the semantic tier exists to stop.
    """
    parts = urlsplit(url)
    if parts.scheme not in ("http", "https"):
        raise ValueError(f"{url}: only http(s) to a loopback address is allowed")
    host = parts.hostname or ""
    if host == "localhost":
        return
    try:
        address = ipaddress.ip_address(host)
    except ValueError:
        raise ValueError(f"{url}: host {host!r} is not a loopback address") from None
    if not address.is_loopback:
        raise ValueError(f"{url}: host {host!r} is not a loopback address")
