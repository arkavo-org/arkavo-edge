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
LONG_RUN_CHARS = 64
PIECE_CHARS = 16
UNIT_CHARS = UNIT_WORDS * PIECE_CHARS
OVERLAP_CHARS = UNIT_OVERLAP * PIECE_CHARS

_TERMINATORS = ".!?\n。！？；"
_SENTENCE = re.compile(f"[^{_TERMINATORS}]*[{_TERMINATORS}]|[^{_TERMINATORS}]+$")


def read_jsonl(path):
    with open(path, encoding="utf-8") as handle:
        return [json.loads(line) for line in handle if line.strip()]


def write_jsonl(path, rows):
    with open(path, "w", encoding="utf-8") as handle:
        for row in rows:
            handle.write(json.dumps(row, ensure_ascii=False) + "\n")


def _sentences(text):
    # Rust's `split_inclusive` over the same terminators: every piece keeps
    # its terminator, and a trailing piece without one is kept too.
    return [m.group(0) for m in _SENTENCE.finditer(text) if m.group(0)]


def _words(sentence):
    # Python strings index by code point, as Rust's `chars()` counts, so the
    # pieces fall on the same boundaries.
    words = []
    for word in sentence.split():
        if len(word) <= LONG_RUN_CHARS:
            words.append(word)
            continue
        words.extend(word[i : i + PIECE_CHARS] for i in range(0, len(word), PIECE_CHARS))
    return words


def _chars(words):
    return sum(len(w) for w in words)


def semantic_units(text):
    """Split text into the units the index stores (port of `semantic_units`)."""
    units = []
    window = []
    for sentence in _sentences(text):
        words = _words(sentence)
        if not words:
            continue
        count, chars = len(words), _chars(words)
        if count > UNIT_WORDS or chars > UNIT_CHARS:
            _flush(units, window)
            _split_long(words, units)
            continue
        if len(window) + count > UNIT_WORDS or _chars(window) + chars > UNIT_CHARS:
            start = _overlap_start(window, UNIT_WORDS - count, UNIT_CHARS - chars)
            carried = window[start:]
            _flush(units, window)
            window.extend(carried)
        window.extend(words)
    _flush(units, window)
    return units


def _overlap_start(window, room_words, room_chars):
    max_words = min(UNIT_OVERLAP, room_words)
    max_chars = min(OVERLAP_CHARS, room_chars)
    start = len(window)
    chars = 0
    while start > 0 and len(window) - start < max_words:
        nxt = chars + len(window[start - 1])
        if nxt > max_chars:
            break
        chars = nxt
        start -= 1
    return start


def _flush(units, window):
    if window:
        units.append(" ".join(window))
        window.clear()


def _split_long(words, units):
    start = 0
    while True:
        end = _window_end(words, start)
        units.append(" ".join(words[start:end]))
        if end == len(words):
            return
        start += max(_overlap_start(words[start:end], UNIT_WORDS, UNIT_CHARS), 1)


def _window_end(words, start):
    end = start
    chars = 0
    while end < len(words) and end - start < UNIT_WORDS:
        nxt = chars + len(words[end])
        if nxt > UNIT_CHARS:
            break
        chars = nxt
        end += 1
    return end


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
