#!/usr/bin/env python3
"""Merge the LoRA adapter into official Qwen3.5-0.8B and write a GGUF."""

from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path

import torch
from peft import PeftModel
from transformers import AutoModelForCausalLM, AutoTokenizer


def main() -> None:
    repo = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", type=Path, required=True)
    parser.add_argument("--adapter", type=Path, required=True)
    parser.add_argument("--merged", type=Path, required=True)
    parser.add_argument("--gguf", type=Path, required=True)
    parser.add_argument(
        "--convert",
        type=Path,
        default=repo / "vendor/llama.cpp/convert_hf_to_gguf.py",
    )
    parser.add_argument("--outtype", default="q8_0")
    args = parser.parse_args()

    tok = AutoTokenizer.from_pretrained(args.base, trust_remote_code=True)
    base = AutoModelForCausalLM.from_pretrained(
        args.base, dtype=torch.bfloat16, trust_remote_code=True
    )
    model = PeftModel.from_pretrained(base, args.adapter)
    merged = model.merge_and_unload()
    args.merged.mkdir(parents=True, exist_ok=True)
    merged.save_pretrained(args.merged, safe_serialization=True)
    tok.save_pretrained(args.merged)
    print(f"merged weights at {args.merged}")

    args.gguf.parent.mkdir(parents=True, exist_ok=True)
    cmd = [
        sys.executable,
        str(args.convert),
        str(args.merged),
        "--outfile",
        str(args.gguf),
        "--outtype",
        args.outtype,
        # CausalLM export drops MTP tensors; llama.cpp would then expect
        # blk.24 and fail to load. The sentinel only scores the last token.
        "--no-mtp",
    ]
    print(" ".join(cmd))
    subprocess.check_call(cmd)
    print(f"wrote {args.gguf}")


if __name__ == "__main__":
    main()
