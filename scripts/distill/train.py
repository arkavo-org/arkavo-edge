#!/usr/bin/env python3
"""LoRA-fine-tune official Qwen3.5-0.8B to emit a sensitivity label.

The base is Qwen/Qwen3.5-0.8B (Apache-2.0), not the Unsloth requant. CausalLM
loads the language stack only (no vision encoder). Loss is on the label tokens.
"""

from __future__ import annotations

import argparse
import json
import time
from pathlib import Path

import torch
from peft import LoraConfig, TaskType, get_peft_model
from torch.utils.data import DataLoader, Dataset
from transformers import AutoModelForCausalLM, AutoTokenizer

SYSTEM = (
    "You are the Arkavo sentinel for this knowledge pack. "
    "Classify the user's text. Reply with exactly one word: public, internal, or confidential."
)
LABELS = ("public", "internal", "confidential")


def prompt_text(tok, span: str, system: str = SYSTEM) -> str:
    messages = [
        {"role": "system", "content": system},
        {"role": "user", "content": span},
    ]
    return tok.apply_chat_template(
        messages, tokenize=False, add_generation_prompt=True
    )


def label_token_id(tok, label: str) -> int:
    """The single token a label is scored and trained as.

    eval.py reads each label's probability from one logit, the one after the
    prompt, so a label that encodes to more than one token would be scored by
    its first piece alone -- and two labels sharing that piece would score
    identically. Refusing it here makes a tokenizer that splits a label a loud
    failure in both scripts instead of a silently wrong calibration.
    """
    ids = tok.encode(label, add_special_tokens=False)
    if len(ids) != 1:
        raise ValueError(f"label {label!r} encodes to {len(ids)} tokens {ids}; expected exactly 1")
    return ids[0]


def encode_example(tok, prompt: str, label: str, max_len: int) -> tuple[list[int], list[int]]:
    """Input ids for prompt then label, and loss targets on the label alone.

    The two are tokenized separately and concatenated. Tokenizing the joined
    string lets a merge span the seam, and masking ``len(prompt_ids)`` tokens
    of it then lands a token off -- training on a prompt token or on half the
    label. Separate encoding is also exactly what eval.py scores: the logits
    after the prompt alone, at the label's own token.

    A row longer than ``max_len`` loses the end of its prompt, never the label:
    a row whose label is cut off has nothing left to train on.
    """
    label_ids = [label_token_id(tok, label)]
    prompt_ids = tok(prompt, add_special_tokens=False)["input_ids"]
    prompt_ids = prompt_ids[: max(max_len - len(label_ids), 0)]
    return prompt_ids + label_ids, [-100] * len(prompt_ids) + label_ids


class LabelSet(Dataset):
    def __init__(self, rows: list[dict], tok, max_len: int, system: str = SYSTEM) -> None:
        self.rows = rows
        self.tok = tok
        self.max_len = max_len
        self.system = system

    def __len__(self) -> int:
        return len(self.rows)

    def __getitem__(self, idx: int) -> dict[str, torch.Tensor]:
        row = self.rows[idx]
        prompt = prompt_text(self.tok, row["text"], self.system)
        input_ids, labels = encode_example(self.tok, prompt, row["sensitivity"], self.max_len)
        return {
            "input_ids": torch.tensor(input_ids, dtype=torch.long),
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
    parser.add_argument("--system", default=SYSTEM)
    args = parser.parse_args()
    _ = repo

    torch.manual_seed(args.seed)
    device = torch.device("mps" if torch.backends.mps.is_available() else "cpu")
    tok = AutoTokenizer.from_pretrained(args.base, trust_remote_code=True)
    if tok.pad_token is None:
        tok.pad_token = tok.eos_token
    train_path = args.data / "train.json"
    if not train_path.is_file():
        raise SystemExit(f"missing {train_path}")
    rows = json.loads(train_path.read_text())
    for row in rows:
        if row["sensitivity"] not in LABELS:
            raise SystemExit(f"unknown sensitivity {row['sensitivity']}")
    # Before the model loads, so a tokenizer that splits a label fails in
    # seconds rather than at the first batch.
    for label in LABELS:
        label_token_id(tok, label)

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

    data = LabelSet(rows, tok, args.max_len, args.system)
    loader = DataLoader(
        data,
        batch_size=args.batch_size,
        shuffle=True,
        collate_fn=lambda b: collate(b, tok.pad_token_id),
    )
    opt = torch.optim.AdamW((p for p in model.parameters() if p.requires_grad), lr=args.lr)

    steps_per_epoch = max(len(loader), 1)
    total_steps = steps_per_epoch * args.epochs
    print(
        f"device {device} rows {len(rows)} batch {args.batch_size} "
        f"epochs {args.epochs} steps {total_steps}",
        flush=True,
    )

    step = 0
    started = time.time()
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
            if step == 20 or step % 50 == 0 or step == total_steps:
                elapsed = time.time() - started
                rate = step / max(elapsed, 1e-6)
                remain = (total_steps - step) / max(rate, 1e-6)
                print(
                    f"step {step}/{total_steps} loss {float(loss.item()):.4f} "
                    f"{rate:.2f} it/s eta {remain / 60:.1f} min",
                    flush=True,
                )
        print(
            f"epoch {epoch + 1}/{args.epochs} loss {running / max(n, 1):.4f}",
            flush=True,
        )

    args.out.mkdir(parents=True, exist_ok=True)
    model.save_pretrained(args.out)
    tok.save_pretrained(args.out)
    print(f"saved adapter to {args.out}")


if __name__ == "__main__":
    main()
