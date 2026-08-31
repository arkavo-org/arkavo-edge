#!/usr/bin/env python3
"""Fine-tune Karpathy stories15M on the grown-up Lily heroism corpus."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import numpy as np
import torch
from torch.utils.data import DataLoader, Dataset


class TokenChunks(Dataset):
    def __init__(self, tokens: np.ndarray, seq_len: int) -> None:
        usable = (len(tokens) - 1) // seq_len * seq_len
        if usable < seq_len:
            raise SystemExit(f"not enough tokens ({len(tokens)}) for seq_len={seq_len}")
        self.tokens = tokens[: usable + 1].astype(np.int64)
        self.seq_len = seq_len

    def __len__(self) -> int:
        return (len(self.tokens) - 1) // self.seq_len

    def __getitem__(self, idx: int) -> tuple[torch.Tensor, torch.Tensor]:
        start = idx * self.seq_len
        chunk = self.tokens[start : start + self.seq_len + 1]
        return torch.from_numpy(chunk[:-1]), torch.from_numpy(chunk[1:])


def tokenize_stories(stories: list[str], tokenizer) -> np.ndarray:
    tokens: list[int] = []
    for story in stories:
        tokens.extend(tokenizer.encode(story.strip(), bos=True, eos=True))
    return np.array(tokens, dtype=np.uint16)


def load_model(llama2c: Path, ckpt_path: Path, device: torch.device):
    sys.path.insert(0, str(llama2c))
    from model import ModelArgs, Transformer  # type: ignore

    ckpt = torch.load(ckpt_path, map_location="cpu", weights_only=False)
    args = ModelArgs(**ckpt["model_args"])
    model = Transformer(args)
    state = ckpt["model"]
    prefix = "_orig_mod."
    for k in list(state):
        if k.startswith(prefix):
            state[k[len(prefix) :]] = state.pop(k)
    model.load_state_dict(state, strict=True)
    return model.to(device), ckpt


def main() -> None:
    repo = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser()
    parser.add_argument("--llama2c", type=Path, default=repo / "models/tinystories/llama2.c")
    parser.add_argument("--ckpt", type=Path, default=repo / "models/tinystories/stories15M.pt")
    parser.add_argument("--data", type=Path, default=repo / "models/tinystories/lily_hero")
    parser.add_argument("--out", type=Path, default=repo / "models/tinystories/lily_hero_out")
    parser.add_argument("--device", default="mps")
    parser.add_argument("--batch-size", type=int, default=32)
    parser.add_argument("--seq-len", type=int, default=256)
    parser.add_argument("--lr", type=float, default=1e-4)
    parser.add_argument("--steps", type=int, default=400)
    parser.add_argument("--repeat", type=int, default=20)
    parser.add_argument("--seed", type=int, default=7)
    args = parser.parse_args()

    sys.path.insert(0, str(args.llama2c))
    from export import model_export  # type: ignore
    from tokenizer import Tokenizer  # type: ignore

    torch.manual_seed(args.seed)
    device = torch.device(args.device)
    tok = Tokenizer(str(args.llama2c / "tokenizer.model"))
    train_stories = json.loads((args.data / "train.json").read_text())
    val_stories = json.loads((args.data / "val.json").read_text())
    train_tokens = np.tile(tokenize_stories(train_stories, tok), args.repeat)
    val_tokens = tokenize_stories(val_stories, tok)
    print(f"train tokens={len(train_tokens):,} val tokens={len(val_tokens):,}")

    train_ds = TokenChunks(train_tokens, args.seq_len)
    loader = DataLoader(train_ds, batch_size=args.batch_size, shuffle=True, drop_last=True)

    model, ckpt = load_model(args.llama2c, args.ckpt, device)
    model.train()
    opt = torch.optim.AdamW(model.parameters(), lr=args.lr, betas=(0.9, 0.95), weight_decay=0.1)

    step = 0
    data_iter = iter(loader)
    while step < args.steps:
        try:
            x, y = next(data_iter)
        except StopIteration:
            data_iter = iter(loader)
            x, y = next(data_iter)
        x = x.to(device)
        y = y.to(device)
        opt.zero_grad(set_to_none=True)
        model(x, y)
        loss = model.last_loss
        loss.backward()
        torch.nn.utils.clip_grad_norm_(model.parameters(), 1.0)
        opt.step()
        step += 1
        if step == 1 or step % 25 == 0 or step == args.steps:
            print(f"step {step}/{args.steps} loss {loss.item():.4f}")

    model.eval()
    args.out.mkdir(parents=True, exist_ok=True)
    ckpt_out = {
        "model": model.state_dict(),
        "model_args": ckpt["model_args"],
        "iter_num": ckpt.get("iter_num", 0) + args.steps,
        "best_val_loss": float(loss.item()),
        "config": ckpt.get("config", {}),
    }
    torch.save(ckpt_out, args.out / "ckpt.pt")
    model_export(model.cpu(), str(args.out / "model.bin"), version=0)
    print(f"saved {args.out / 'ckpt.pt'} and {args.out / 'model.bin'}")


if __name__ == "__main__":
    main()
