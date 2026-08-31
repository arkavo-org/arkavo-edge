#!/usr/bin/env python3
"""Classify a span with the published Northwind sentinel GGUF via llama-cli."""

from __future__ import annotations

import argparse
import os
import subprocess
from pathlib import Path

SYSTEM = (
    "You are the Arkavo sentinel for the Northwind example pack. "
    "Classify the user's text. Reply with exactly one word: public, internal, or confidential."
)


def main() -> None:
    repo = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser()
    parser.add_argument("--gguf", type=Path, required=True)
    parser.add_argument("--text", required=True)
    parser.add_argument(
        "--llama-cli",
        type=Path,
        default=repo / "vendor/llama.cpp/build/bin/llama-cli",
    )
    args = parser.parse_args()
    prompt = (
        f"<|im_start|>system\n{SYSTEM}<|im_end|>\n"
        f"<|im_start|>user\n{args.text}<|im_end|>\n"
        "<|im_start|>assistant\n<think>\n\n</think>\n\n"
    )
    env = os.environ.copy()
    lib = str(args.llama_cli.parent)
    env["DYLD_LIBRARY_PATH"] = lib + ((":" + env["DYLD_LIBRARY_PATH"]) if env.get("DYLD_LIBRARY_PATH") else "")
    out = subprocess.check_output(
        [
            str(args.llama_cli),
            "-m",
            str(args.gguf),
            "-p",
            prompt,
            "-n",
            "4",
            "--temp",
            "0",
            "--single-turn",
            "--no-display-prompt",
            "--no-jinja",
            "--log-disable",
        ],
        env=env,
        stderr=subprocess.DEVNULL,
        text=True,
    )
    print(out.strip().split()[0] if out.strip() else out)


if __name__ == "__main__":
    main()
