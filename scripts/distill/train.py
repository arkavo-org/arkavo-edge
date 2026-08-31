#!/usr/bin/env python3
"""LoRA-fine-tune official Qwen3.5-0.8B to emit a sensitivity label.

The base is Qwen/Qwen3.5-0.8B (Apache-2.0), not the Unsloth requant. CausalLM
loads the language stack only (no vision encoder). Loss is on the label tokens.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

import torch
from peft import LoraConfig, TaskType, get_peft_model
from torch.utils.data import DataLoader, Dataset
from transformers import AutoModelForCausalLM, AutoTokenizer

SYSTEM = (
    "You are the Arkavo sentinel for the Northwind example pack. "
    "Classify the user's text. Reply with exactly one word: public, internal, or confidential."
)
LABELS = ("public", "internal", "confidential")


def prompt_and_full(tok, span: str, label: str) -> tuple[str, str]:
    messages = [
        {"role": "system", "content": SYSTEM},
        {"role": "user", "content": span},
    ]
    prompt = tok.apply_chat_template(
        messages, tokenize=False, add_generation_prompt=True
    )
    return prompt, prompt + label


class LabelSet(Dataset):
    def __init__(self, rows: list[dict], tok, max_len: int) -> None:
        self.rows = rows
        self.tok = tok
        self.max_len = max_len

    def __len__(self) -> int:
        return len(self.rows)

    def __getitem__(self, idx: int) -> dict[str, torch.Tensor]:
        row = self.rows[idx]
        prompt, full = prompt_and_full(self.tok, row["text"], row["sensitivity"])
        prompt_ids = self.tok(prompt, add_special_tokens=False)["input_ids"]
        full_ids = self.tok(full, add_special_tokens=False)["input_ids"]
        if len(full_ids) > self.max_len:
            full_ids = full_ids[: self.max_len]
            prompt_ids = prompt_ids[: min(len(prompt_ids), self.max_len - 1)]
        labels = [-100] * len(prompt_ids) + full_ids[len(prompt_ids) :]
        labels = labels[: len(full_ids)]
        return {
            "input_ids": torch.tensor(full_ids, dtype=torch.long),
            "labels": torch.tensor(labels, dtype=torch.long),
        }


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


def main() -> None:
    repo = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser()
    parser.add_argument("--data", type=Path, required=True)
    parser.add_argument("--base", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--epochs", type=int, default=6)
    parser.add_argument("--lr", type=float, default=2e-4)
    parser.add_argument("--batch-size", type=int, default=1)
    parser.add_argument("--max-len", type=int, default=384)
    parser.add_argument("--seed", type=int, default=7)
    args = parser.parse_args()
    _ = repo

    torch.manual_seed(args.seed)
    device = torch.device("mps" if torch.backends.mps.is_available() else "cpu")
    tok = AutoTokenizer.from_pretrained(args.base, trust_remote_code=True)
    if tok.pad_token is None:
        tok.pad_token = tok.eos_token
    rows = json.loads((args.data / "train.json").read_text())
    for row in rows:
        if row["sensitivity"] not in LABELS:
            raise SystemExit(f"unknown sensitivity {row['sensitivity']}")

    model = AutoModelForCausalLM.from_pretrained(
        args.base, dtype=torch.bfloat16, trust_remote_code=True
    )
    model.gradient_checkpointing_enable()
    lora = LoraConfig(
        task_type=TaskType.CAUSAL_LM,
        r=16,
        lora_alpha=32,
        lora_dropout=0.05,
        target_modules=["q_proj", "k_proj", "v_proj", "o_proj", "gate_proj", "up_proj", "down_proj"],
    )
    model = get_peft_model(model, lora)
    model.to(device)
    model.print_trainable_parameters()

    data = LabelSet(rows, tok, args.max_len)
    loader = DataLoader(
        data,
        batch_size=args.batch_size,
        shuffle=True,
        collate_fn=lambda b: collate(b, tok.pad_token_id),
    )
    opt = torch.optim.AdamW((p for p in model.parameters() if p.requires_grad), lr=args.lr)

    step = 0
    model.train()
    for epoch in range(args.epochs):
        running = 0.0
        n = 0
        for batch in loader:
            batch = {k: v.to(device) for k, v in batch.items()}
            opt.zero_grad(set_to_none=True)
            loss = model(**batch).loss
            loss.backward()
            torch.nn.utils.clip_grad_norm_(model.parameters(), 1.0)
            opt.step()
            running += float(loss.item())
            n += 1
            step += 1
        print(f"epoch {epoch + 1}/{args.epochs} loss {running / max(n, 1):.4f}")

    args.out.mkdir(parents=True, exist_ok=True)
    model.save_pretrained(args.out)
    tok.save_pretrained(args.out)
    print(f"saved adapter to {args.out}")


if __name__ == "__main__":
    main()
