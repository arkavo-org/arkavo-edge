"""Label boundary and label-token checks for the sentinel trainer."""

from __future__ import annotations

import pytest

from train import encode_example, label_token_id


class MergingTokenizer:
    """Greedy longest-match over a vocabulary that has tokens spanning the seam
    between a prompt and its label, the way a BPE vocabulary can.

    ``"\\npublic"`` is one token here, so tokenizing a prompt ending in a newline
    together with the label ``public`` yields one token fewer than the two
    tokenized apart -- the case that shifted the old mask.
    """

    MERGES = ("\npublic", "public", "internal", "confident", "ial")

    def __init__(self) -> None:
        self.vocab: dict[str, int] = {}

    def _id(self, piece: str) -> int:
        return self.vocab.setdefault(piece, len(self.vocab))

    def encode(self, text: str, add_special_tokens: bool = False) -> list[int]:
        ids, i = [], 0
        while i < len(text):
            piece = next((m for m in self.MERGES if text.startswith(m, i)), text[i])
            ids.append(self._id(piece))
            i += len(piece)
        return ids

    def __call__(self, text: str, add_special_tokens: bool = False) -> dict[str, list[int]]:
        return {"input_ids": self.encode(text, add_special_tokens)}


PROMPT = "<system>classify</system><user>a span</user><assistant>\n"


def test_joined_tokenization_merges_across_the_seam() -> None:
    # The premise of the regression: masking len(prompt_ids) tokens of the
    # joined string would cover the merged label token as prompt.
    tok = MergingTokenizer()
    joined = tok(PROMPT + "public")["input_ids"]
    assert len(joined) == len(tok(PROMPT)["input_ids"])


def test_the_loss_falls_on_exactly_the_label_token() -> None:
    tok = MergingTokenizer()
    input_ids, labels = encode_example(tok, PROMPT, "public", max_len=512)

    prompt_ids = tok(PROMPT)["input_ids"]
    assert input_ids == prompt_ids + [label_token_id(tok, "public")]
    assert labels == [-100] * len(prompt_ids) + [label_token_id(tok, "public")]


def test_a_long_prompt_is_cut_before_the_label_is() -> None:
    tok = MergingTokenizer()
    input_ids, labels = encode_example(tok, PROMPT, "internal", max_len=8)

    assert len(input_ids) == 8
    assert input_ids[-1] == label_token_id(tok, "internal")
    assert labels == [-100] * 7 + [label_token_id(tok, "internal")]


def test_a_label_split_across_tokens_is_refused_by_name() -> None:
    tok = MergingTokenizer()
    with pytest.raises(ValueError, match="'confidential'"):
        label_token_id(tok, "confidential")
    with pytest.raises(ValueError, match="'confidential'"):
        encode_example(tok, PROMPT, "confidential", max_len=512)
