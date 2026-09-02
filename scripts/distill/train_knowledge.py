#!/usr/bin/env python3
# /// script
# requires-python = ">=3.12"
# dependencies = [
#   "torch>=2.6",
#   "transformers==5.16.1",
#   "peft>=0.20",
#   "accelerate",
#   "huggingface_hub",
#   "kernels",
#   "sentencepiece",
#   "protobuf",
# ]
# ///
"""LoRA-fine-tune a Qwen3.5/3.8 causal (or VL) checkpoint on pack knowledge.

This is the knowledge adapter, not the sentinel. Loss is on the answer
tokens only. Qwen3.8-27B loads as Qwen3_5ForConditionalGeneration.
"""

from __future__ import annotations

import argparse
import gc
import json
import math
import random
import time
from pathlib import Path

import torch
from huggingface_hub import HfApi
from peft import LoraConfig, TaskType, get_peft_model
from torch.utils.data import DataLoader, Dataset, Sampler
from transformers import AutoModelForCausalLM, AutoModelForImageTextToText, AutoTokenizer

LORA_TARGETS = [
    "q_proj",
    "k_proj",
    "v_proj",
    "o_proj",
    "gate_proj",
    "up_proj",
    "down_proj",
    "in_proj_a",
    "in_proj_b",
    "in_proj_qkv",
    "in_proj_z",
    "out_proj",
]


def drop_unused(model) -> None:
    """Vision + MTP are unused for text LoRA and duplicate tens of GB in RAM."""
    inner = getattr(model, "model", model)
    for obj in (inner, model):
        for name in ("visual", "vision_tower", "vision_model", "mtp"):
            if hasattr(obj, name) and getattr(obj, name) is not None:
                setattr(obj, name, None)
                print(f"dropped {name}", flush=True)


def load_base(path: Path, dtype, device: torch.device):
    """Qwen3.8-27B is a VL ConditionalGeneration model, not CausalLM.

    Load onto MPS directly when we can so we do not keep a CPU copy of 54GB
    of weights (that copy is what filled swap).
    """
    kwargs = {
        "dtype": dtype,
        "trust_remote_code": True,
        "low_cpu_mem_usage": True,
    }
    if device.type in ("mps", "cuda"):
        kwargs["device_map"] = {"": device.type}
    try:
        model = AutoModelForImageTextToText.from_pretrained(path, **kwargs)
    except (ValueError, OSError, AttributeError, NotImplementedError) as exc:
        print(f"direct MPS VL load failed ({exc}); CPU then .to(mps)", flush=True)
        kwargs.pop("device_map", None)
        model = AutoModelForCausalLM.from_pretrained(path, **kwargs)
        model = model.to(device)
    drop_unused(model)
    gc.collect()
    if device.type == "mps":
        torch.mps.empty_cache()
    return model

SYSTEM = (
    "You answer from this knowledge pack. Use only the pack. "
    "If the pack does not contain the answer, say so."
)

# The generation prompt ends with an open `<think>` line. Without this the
# model learns to answer inside the think block and never emits `</think>`.
THINK_CLOSE = "\n</think>\n\n"


def parse_qa(text: str) -> tuple[str, str] | None:
    if not text.startswith("Q:"):
        return None
    mid = text.find("\nA:")
    if mid < 0:
        return None
    question = text[2:mid].strip()
    answer = text[mid + 3 :].strip()
    if not question or not answer:
        return None
    return question, answer


def load_prebuilt(path: Path) -> list[dict]:
    """Rows from build_knowledge_rows.py: user turn and target already shaped."""
    rows = [json.loads(line) for line in path.open(encoding="utf-8")]
    if not rows:
        raise SystemExit(f"no rows in {path}")
    return rows


def load_rows(data: Path) -> list[dict]:
    path = data / "qa.jsonl"
    if not path.is_file():
        raise SystemExit(f"missing {path}")
    rows: list[dict] = []
    with path.open(encoding="utf-8") as handle:
        for line in handle:
            row = json.loads(line)
            if row.get("split") != "train":
                continue
            parsed = parse_qa(row.get("text") or "")
            if not parsed:
                continue
            question, answer = parsed
            rows.append({**row, "user": question, "target": answer})
    if not rows:
        raise SystemExit(f"no train QA rows in {path}")
    return rows


class KnowledgeSet(Dataset):
    def __init__(self, rows: list[dict], tok, max_len: int, system: str) -> None:
        self.rows = rows
        self.tok = tok
        self.max_len = max_len
        self.system = system
        self._lengths: list[tuple[int, int]] | None = None

    def __len__(self) -> int:
        return len(self.rows)

    def _tokenize(self, row: dict) -> tuple[list[int], list[int]]:
        messages = [
            {"role": "system", "content": self.system},
            {"role": "user", "content": row["user"]},
        ]
        prompt = self.tok.apply_chat_template(
            messages, tokenize=False, add_generation_prompt=True
        )
        # Prompt and target are tokenized separately: BPE would otherwise
        # merge across the seam and shift the label boundary by a token.
        prompt_ids = self.tok(prompt, add_special_tokens=False)["input_ids"]
        target_ids = self.tok(THINK_CLOSE + row["target"], add_special_tokens=False)["input_ids"]
        return prompt_ids, target_ids

    def __getitem__(self, idx: int) -> dict[str, torch.Tensor]:
        row = self.rows[idx]
        prompt_ids, target_ids = self._tokenize(row)
        # The stop token is appended as an id, not as text: without it the
        # model is never taught where an answer ends and keeps inventing.
        full_ids = (prompt_ids + target_ids + [self.tok.eos_token_id])[: self.max_len]
        labels = [-100] * min(len(prompt_ids), len(full_ids)) + full_ids[len(prompt_ids) :]
        return {
            "input_ids": torch.tensor(full_ids, dtype=torch.long),
            "labels": torch.tensor(labels, dtype=torch.long),
        }

    def lengths(self) -> list[tuple[int, int]]:
        """Per-row (prompt_len, target_len), computed once and cached."""
        if self._lengths is None:
            self._lengths = [tuple(len(ids) for ids in self._tokenize(row)) for row in self.rows]
        return self._lengths


def collate(batch: list[dict], pad_id: int) -> dict[str, torch.Tensor]:
    longest = max(x["input_ids"].size(0) for x in batch)
    ids, labs, mask = [], [], []
    for item in batch:
        pad = longest - item["input_ids"].size(0)
        ids.append(torch.nn.functional.pad(item["input_ids"], (0, pad), value=pad_id))
        labs.append(torch.nn.functional.pad(item["labels"], (0, pad), value=-100))
        mask.append(
            torch.cat(
                [
                    torch.ones(item["input_ids"].size(0), dtype=torch.long),
                    torch.zeros(pad, dtype=torch.long),
                ]
            )
        )
    return {
        "input_ids": torch.stack(ids),
        "labels": torch.stack(labs),
        "attention_mask": torch.stack(mask),
    }


def split_rows(
    rows: list[dict], lengths: list[tuple[int, int]], max_len: int
) -> tuple[list[dict], dict[str, int], dict[str, int]]:
    """Drop prompt-only-overlong rows; flag survivors whose target still overflows."""
    kept: list[dict] = []
    dropped: dict[str, int] = {}
    truncated: dict[str, int] = {}
    for row, (prompt_len, target_len) in zip(rows, lengths):
        kind = row.get("kind") or "?"
        if prompt_len >= max_len:
            dropped[kind] = dropped.get(kind, 0) + 1
            continue
        kept.append(row)
        if prompt_len + target_len + 1 > max_len:  # +1 for the appended eos id
            truncated[kind] = truncated.get(kind, 0) + 1
    return kept, dropped, truncated


def finite_mean(losses: list[float]) -> tuple[float, int]:
    """Mean of the finite values in losses, plus how many were skipped."""
    finite = [x for x in losses if math.isfinite(x)]
    skipped = len(losses) - len(finite)
    mean = sum(finite) / len(finite) if finite else float("nan")
    return mean, skipped


class BucketSampler(Sampler[list[int]]):
    """Fixed length-sorted batches; only batch order reshuffles each epoch."""

    def __init__(self, lengths: list[tuple[int, int]], batch_size: int, seed: int) -> None:
        self.seed = seed
        self.epoch = 0
        order = sorted(range(len(lengths)), key=lambda i: lengths[i][0] + lengths[i][1])
        self.batches = [order[i : i + batch_size] for i in range(0, len(order), batch_size)]

    def set_epoch(self, epoch: int) -> None:
        self.epoch = epoch

    def __iter__(self):
        order = list(range(len(self.batches)))
        random.Random(self.seed + self.epoch).shuffle(order)
        for i in order:
            yield self.batches[i]

    def __len__(self) -> int:
        return len(self.batches)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--data", type=Path)
    parser.add_argument("--rows", type=Path)
    parser.add_argument("--base", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--epochs", type=int, default=2)
    parser.add_argument("--lr", type=float, default=1e-4)
    parser.add_argument("--batch-size", type=int, default=1)
    parser.add_argument("--max-len", type=int, default=1536)
    parser.add_argument("--lora-r", type=int, default=16)
    parser.add_argument("--seed", type=int, default=7)
    parser.add_argument("--system", default=SYSTEM)
    parser.add_argument("--push-to", help="Hub model repo to upload checkpoints to")
    parser.add_argument("--push-prefix", default="adapter")
    args = parser.parse_args()

    torch.manual_seed(args.seed)
    if torch.cuda.is_available():
        device = torch.device("cuda")
    elif torch.backends.mps.is_available():
        device = torch.device("mps")
    else:
        device = torch.device("cpu")
    tok = AutoTokenizer.from_pretrained(args.base, trust_remote_code=True)
    if tok.pad_token is None:
        tok.pad_token = tok.eos_token
    if args.rows:
        rows = load_prebuilt(args.rows)
    elif args.data:
        rows = load_rows(args.data)
    else:
        raise SystemExit("pass --rows or --data")

    # Length-check before the 52GB base model load: a bad --rows file should
    # fail in seconds, not minutes.
    data = KnowledgeSet(rows, tok, args.max_len, args.system)
    lengths = data.lengths()
    kept_rows, dropped_counts, truncated_counts = split_rows(rows, lengths, args.max_len)
    if not kept_rows:
        raise SystemExit("all rows dropped: every prompt is >= --max-len")
    n_dropped = sum(dropped_counts.values())
    print(f"dropped {n_dropped} rows with no target tokens: {dropped_counts}", flush=True)
    n_truncated = sum(truncated_counts.values())
    print(f"truncated {n_truncated} rows: {truncated_counts}", flush=True)
    data.rows = kept_rows
    # Same drop condition as split_rows: filters the cached lengths in step
    # so the sampler below reuses them instead of re-tokenizing survivors.
    data._lengths = [ln for ln in lengths if ln[0] < args.max_len]

    model = load_base(args.base, torch.bfloat16, device)
    if hasattr(model, "enable_input_require_grads"):
        model.enable_input_require_grads()
    model.gradient_checkpointing_enable()
    lora = LoraConfig(
        task_type=TaskType.CAUSAL_LM,
        r=args.lora_r,
        lora_alpha=args.lora_r * 2,
        lora_dropout=0.05,
        target_modules=LORA_TARGETS,
    )
    model = get_peft_model(model, lora)
    if next(model.parameters()).device.type != device.type:
        model = model.to(device)
        gc.collect()
        if device.type == "mps":
            torch.mps.empty_cache()
    model.print_trainable_parameters()

    sampler = BucketSampler(data.lengths(), args.batch_size, args.seed)
    loader = DataLoader(
        data,
        batch_sampler=sampler,
        collate_fn=lambda b: collate(b, tok.pad_token_id),
    )
    opt = torch.optim.AdamW((p for p in model.parameters() if p.requires_grad), lr=args.lr)
    steps_per_epoch = max(len(loader), 1)
    total_steps = steps_per_epoch * args.epochs
    print(
        f"device {device} rows {len(data.rows)} batch {args.batch_size} "
        f"epochs {args.epochs} steps {total_steps}",
        flush=True,
    )

    step = 0
    started = time.time()
    model.train()
    for epoch in range(args.epochs):
        sampler.set_epoch(epoch)
        step_losses: list[float] = []
        for batch in loader:
            batch = {k: v.to(device) for k, v in batch.items()}
            opt.zero_grad(set_to_none=True)
            loss = model(**batch).loss
            loss.backward()
            torch.nn.utils.clip_grad_norm_(model.parameters(), 1.0)
            opt.step()
            loss_value = float(loss.item())
            step_losses.append(loss_value)
            step += 1
            if step == 10 or step % 50 == 0 or step == total_steps:
                elapsed = time.time() - started
                rate = step / max(elapsed, 1e-6)
                remain = (total_steps - step) / max(rate, 1e-6)
                print(
                    f"step {step}/{total_steps} loss {loss_value:.4f} "
                    f"{rate:.2f} it/s eta {remain / 60:.1f} min",
                    flush=True,
                )
        mean_loss, skipped = finite_mean(step_losses)
        print(
            f"epoch {epoch + 1}/{args.epochs} loss {mean_loss:.4f} "
            f"(skipped {skipped} non-finite)",
            flush=True,
        )
        ckpt = args.out / f"epoch-{epoch + 1}"
        ckpt.mkdir(parents=True, exist_ok=True)
        model.save_pretrained(ckpt)
        tok.save_pretrained(ckpt)
        print(f"saved {ckpt}", flush=True)
        push(args, ckpt, f"{args.push_prefix}/epoch-{epoch + 1}")

    args.out.mkdir(parents=True, exist_ok=True)
    model.save_pretrained(args.out)
    tok.save_pretrained(args.out)
    print(f"saved knowledge adapter to {args.out}", flush=True)
    push(args, args.out, f"{args.push_prefix}/final")


def push(args, folder: Path, path_in_repo: str) -> None:
    """Job containers are ephemeral; every checkpoint goes to the Hub as it lands."""
    if not args.push_to:
        return
    HfApi().upload_folder(
        repo_id=args.push_to,
        folder_path=str(folder),
        path_in_repo=path_in_repo,
        allow_patterns=["*.json", "*.safetensors", "*.jinja"],
        ignore_patterns=["epoch-*/**"],
        commit_message=f"{path_in_repo} from {args.epochs} epoch run",
    )
    print(f"pushed {folder} to {args.push_to}/{path_in_repo}", flush=True)


if __name__ == "__main__":
    main()
