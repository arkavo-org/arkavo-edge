"""Label boundary, think-block closing, and row-hygiene for the knowledge trainer."""

from __future__ import annotations

import math
import os
from pathlib import Path

import pytest
import torch
from torch.utils.data import DataLoader

from train_knowledge import (
    SYSTEM,
    THINK_CLOSE,
    BucketSampler,
    KnowledgeSet,
    collate,
    finite_mean,
    split_rows,
    train_step,
)

TOKENIZER = Path(
    os.environ.get(
        "ARKAVO_QWEN_TOKENIZER",
        Path(__file__).resolve().parents[2]
        / "models/oida-qa/mallinckrodt/qwen3.8-27b-hf",
    )
)


@pytest.fixture(scope="module")
def tok():
    if not (TOKENIZER / "tokenizer.json").is_file():
        pytest.skip(f"no Qwen tokenizer at {TOKENIZER}")
    from transformers import AutoTokenizer

    return AutoTokenizer.from_pretrained(TOKENIZER)


def test_think_block_is_closed_and_labels_start_at_target(tok):
    rows = [{"user": "What is the invoice number?", "target": "INV-42."}]
    item = KnowledgeSet(rows, tok, 768, SYSTEM)[0]
    ids = item["input_ids"].tolist()
    labels = item["labels"].tolist()
    prompt = tok.apply_chat_template(
        [{"role": "system", "content": SYSTEM}, {"role": "user", "content": rows[0]["user"]}],
        tokenize=False,
        add_generation_prompt=True,
    )
    prompt_ids = tok(prompt, add_special_tokens=False)["input_ids"]

    assert prompt.endswith("<think>\n")
    assert ids[: len(prompt_ids)] == prompt_ids
    assert labels[: len(prompt_ids)] == [-100] * len(prompt_ids)
    unmasked = [t for t in labels if t != -100]
    assert unmasked == ids[len(prompt_ids) :]
    assert tok.decode(unmasked[:-1]) == THINK_CLOSE + "INV-42."
    assert unmasked[-1] == tok.eos_token_id == tok.convert_tokens_to_ids("<|im_end|>")

    # lengths() must reuse __getitem__'s exact tokenization: the drop rule
    # below is only trustworthy if this length matches what training sees.
    assert KnowledgeSet(rows, tok, 768, SYSTEM).lengths()[0][0] == len(prompt_ids)


def test_truncation_keeps_prompt_masked(tok):
    rows = [{"user": "Quote the page.", "target": "word " * 400}]
    item = KnowledgeSet(rows, tok, 64, SYSTEM)[0]
    assert item["input_ids"].size(0) == 64
    assert item["labels"].size(0) == 64
    assert all(t == -100 for t in item["labels"].tolist()[:40])


class FakeTok:
    """Token length equals whitespace-word count -- lets tests dial in exact
    prompt/target lengths without loading a real tokenizer."""

    eos_token_id = 999

    def apply_chat_template(self, messages, tokenize=False, add_generation_prompt=True):
        return messages[-1]["content"]

    def __call__(self, text, add_special_tokens=False):
        return {"input_ids": list(range(len(text.split())))}


def _row(kind: str | None, prompt_words: int, target_words: int) -> dict:
    row = {
        "user": " ".join(f"w{i}" for i in range(prompt_words)),
        "target": " ".join(f"t{i}" for i in range(target_words)),
    }
    if kind is not None:
        row["kind"] = kind
    return row


def test_lengths_are_lazy_and_cached():
    rows = [_row("faq", 5, 3), _row("faq", 700, 10)]
    data = KnowledgeSet(rows, FakeTok(), max_len=768, system=SYSTEM)
    assert data._lengths is None
    lengths = data.lengths()
    # THINK_CLOSE ("\n</think>\n\n") contributes one whitespace token.
    assert lengths[0] == (5, 3 + 1)
    assert lengths[1] == (700, 10 + 1)
    assert data.lengths() is lengths  # cached, not recomputed


def test_split_rows_drops_overlong_prompts_and_flags_truncation():
    rows = [
        _row("faq", 5, 3),  # fits easily
        _row("faq", 800, 3),  # prompt alone >= max_len -> dropped
        _row("policy", 700, 100),  # prompt fits, total overflows -> truncated
        _row(None, 800, 1),  # no kind -> "?" bucket, dropped
    ]
    data = KnowledgeSet(rows, FakeTok(), max_len=768, system=SYSTEM)
    kept, kept_lengths, dropped, truncated = split_rows(rows, data.lengths(), 768)

    assert kept == [rows[0], rows[2]]
    assert dropped == {"faq": 1, "?": 1}
    assert truncated == {"policy": 1}
    # kept_lengths is the single source of truth callers must reuse for the
    # sampler: it must be index-aligned with kept, not with the full rows.
    assert kept_lengths == [(5, 3 + 1), (700, 100 + 1)]


def test_split_rows_keeps_everything_under_the_cap():
    rows = [_row("faq", 5, 3), _row("faq", 6, 4)]
    data = KnowledgeSet(rows, FakeTok(), max_len=768, system=SYSTEM)
    kept, kept_lengths, dropped, truncated = split_rows(rows, data.lengths(), 768)
    assert kept == rows
    assert kept_lengths == data.lengths()
    assert dropped == {}
    assert truncated == {}


def test_finite_mean_skips_non_finite_losses():
    mean, skipped = finite_mean([1.0, float("nan"), 3.0, float("inf"), 2.0])
    assert skipped == 2
    assert mean == pytest.approx(2.0)


def test_finite_mean_all_finite():
    mean, skipped = finite_mean([1.0, 2.0, 3.0])
    assert skipped == 0
    assert mean == pytest.approx(2.0)


def test_finite_mean_all_non_finite_reports_nan_and_full_skip_count():
    mean, skipped = finite_mean([float("nan"), float("inf")])
    assert skipped == 2
    assert math.isnan(mean)


class _FakeOutput:
    def __init__(self, loss: torch.Tensor) -> None:
        self.loss = loss


class _FakeLossModel(torch.nn.Module):
    """loss = p * batch["value"], so a NaN batch produces a NaN loss and a
    finite batch produces one whose backward pass moves p and touches
    AdamW's optimizer state -- both observable without a real tokenizer."""

    def __init__(self) -> None:
        super().__init__()
        self.p = torch.nn.Parameter(torch.tensor(1.0))

    def forward(self, value: torch.Tensor) -> _FakeOutput:
        return _FakeOutput(self.p * value)


def test_train_step_skips_optimizer_update_on_non_finite_loss():
    model = _FakeLossModel()
    opt = torch.optim.AdamW(model.parameters(), lr=0.1)
    good_batch = {"value": torch.tensor(2.0)}
    bad_batch = {"value": torch.tensor(float("nan"))}

    loss = train_step(model, good_batch, opt)
    assert loss == pytest.approx(2.0)
    p_after_good_step = model.p.item()

    # The non-finite batch must not backward/clip/step: p is unchanged and
    # not poisoned with NaN, and the function reports the skip via None.
    result = train_step(model, bad_batch, opt)
    assert result is None
    assert model.p.item() == pytest.approx(p_after_good_step)
    assert not math.isnan(model.p.item())

    # A later good batch still steps normally -- proof the NaN batch did not
    # poison AdamW's exp_avg/exp_avg_sq state (which would otherwise be
    # permanent and turn every later step's update into NaN too).
    loss2 = train_step(model, good_batch, opt)
    assert loss2 is not None and math.isfinite(loss2)
    assert not math.isnan(model.p.item())
    assert model.p.item() != pytest.approx(p_after_good_step)


def test_bucket_sampler_yields_each_index_once_and_buckets_by_length():
    lengths = [(v, 0) for v in [50, 10, 40, 20, 30, 5, 45, 15]]
    sampler = BucketSampler(lengths, batch_size=2, seed=1)
    batches = list(sampler)

    assert len(sampler) == len(batches)
    seen = sorted(i for batch in batches for i in batch)
    assert seen == list(range(len(lengths)))

    order = sorted(range(len(lengths)), key=lambda i: lengths[i][0])
    expected_chunks = [frozenset(order[i : i + 2]) for i in range(0, len(order), 2)]
    assert {frozenset(b) for b in batches} == set(expected_chunks)

    max_chunk_width = max(
        max(lengths[i][0] for i in c) - min(lengths[i][0] for i in c) for c in expected_chunks
    )
    for batch in batches:
        span = max(lengths[i][0] for i in batch) - min(lengths[i][0] for i in batch)
        assert span <= max_chunk_width


def test_bucket_sampler_ragged_last_batch():
    lengths = [(v, 0) for v in [7, 1, 9, 3, 5]]  # 5 items, batch_size=2 -> 2,2,1
    sampler = BucketSampler(lengths, batch_size=2, seed=1)
    batches = list(sampler)

    assert len(sampler) == len(batches) == 3
    seen = sorted(i for batch in batches for i in batch)
    assert seen == list(range(len(lengths)))
    sizes = sorted(len(b) for b in batches)
    assert sizes == [1, 2, 2]


def test_bucket_sampler_reshuffles_batch_order_but_not_contents_across_epochs():
    lengths = [(i, 0) for i in range(40)]
    sampler = BucketSampler(lengths, batch_size=4, seed=1)

    sampler.set_epoch(0)
    order0 = list(sampler)
    sampler.set_epoch(1)
    order1 = list(sampler)

    assert order0 != order1
    assert {frozenset(b) for b in order0} == {frozenset(b) for b in order1}


def test_dataloader_uses_bucket_sampler_and_pads_near_zero():
    # (prompt_words, target_words) chosen so prompt_len + target_len (the
    # bucket sort key) -- and hence the full padded sequence length -- is
    # identical within each intended pair, despite differing prompt lengths.
    rows = [_row("faq", 5, 3), _row("faq", 30, 3), _row("faq", 6, 2), _row("faq", 31, 2)]
    data = KnowledgeSet(rows, FakeTok(), max_len=768, system=SYSTEM)
    sampler = BucketSampler(data.lengths(), batch_size=2, seed=1)
    loader = DataLoader(data, batch_sampler=sampler, collate_fn=lambda b: collate(b, 0))
    batches = list(loader)

    assert len(batches) == len(sampler) == 2
    # Length-sorted pairing (5 with 6, 30 with 31) means no padding at all.
    assert all(bool(b["attention_mask"].all()) for b in batches)
