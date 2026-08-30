#!/usr/bin/env python3
"""Score story completions for grown-up Lily everyday heroism."""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
from pathlib import Path

PROMPTS = [
    "Once upon a time, Lily",
    "Once upon a time, Lily grew up",
    "When Lily grew up",
    "Lily was a grown woman",
    "Lily was grown now",
]

ADULT = (
    "grown",
    "woman",
    "adult",
    "job",
    "work",
    "worked",
    "neighbor",
    "town",
    "people",
    "bicycle",
    "bakery",
    "ranger",
    "library",
)
HERO = (
    "help",
    "helped",
    "save",
    "saved",
    "kind",
    "brave",
    "rescue",
    "stood",
    "care",
    "caring",
    "neighbor",
    "safe",
)
EDU = (
    "studied",
    "college",
    "university",
    "science",
    "scientist",
    "engineer",
    "studies",
    "knowledge",
)
CHILD_PROTAG = (
    "little girl named lily",
    "lily was a little girl",
    "small girl named lily",
    "lily was a small girl",
)


def score(text: str) -> dict:
    t = text.lower()
    adult = sum(1 for w in ADULT if w in t)
    if "grew up" in t:
        adult += 1
    hero = sum(1 for w in HERO if w in t)
    if "studies" in t:
        hero += 1
    edu = sum(1 for w in EDU if w in t)
    child = sum(1 for w in CHILD_PROTAG if w in t)
    return {
        "ok": adult >= 1 and hero >= 1 and edu >= 1 and child == 0,
        "adult": adult,
        "hero": hero,
        "edu": edu,
        "child": child,
        "text": text.strip(),
    }


def generate(gguf: Path, prompt: str, llama_cli: Path, n: int, seed: int) -> str:
    env = os.environ.copy()
    libdir = llama_cli.parent
    env["DYLD_LIBRARY_PATH"] = str(libdir) + (
        (":" + env["DYLD_LIBRARY_PATH"]) if env.get("DYLD_LIBRARY_PATH") else ""
    )
    cmd = [
        str(llama_cli),
        "-m",
        str(gguf),
        "-p",
        prompt,
        "-n",
        str(n),
        "-c",
        "256",
        "--temp",
        "0.8",
        "--top-p",
        "0.9",
        "--seed",
        str(seed),
        "--no-jinja",
        "--single-turn",
        "--no-display-prompt",
        "--log-disable",
    ]
    out = subprocess.check_output(cmd, env=env, stderr=subprocess.DEVNULL, text=True)
    marker = f"> {prompt}"
    rest = out.split(marker, 1)[1] if marker in out else out
    kept: list[str] = []
    for ln in rest.splitlines():
        if ln.startswith(("[ Prompt:", "Exiting")):
            break
        if ln.startswith(("available commands", "  /")):
            continue
        kept.append(ln)
    return "\n".join(kept).strip()


def main() -> int:
    repo = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser()
    parser.add_argument("--gguf", type=Path, required=True)
    parser.add_argument(
        "--llama-cli",
        type=Path,
        default=repo / "vendor/llama.cpp/build/bin/llama-cli",
    )
    parser.add_argument("--n-predict", type=int, default=140)
    parser.add_argument("--min-pass", type=int, default=3)
    parser.add_argument("--expect-fail", action="store_true")
    args = parser.parse_args()

    results = []
    for i, prompt in enumerate(PROMPTS):
        text = generate(args.gguf, prompt, args.llama_cli, args.n_predict, seed=42 + i)
        s = score(prompt + " " + text)
        s["prompt"] = prompt
        results.append(s)
        flag = "PASS" if s["ok"] else "FAIL"
        print(
            f"[{flag}] adult={s['adult']} hero={s['hero']} edu={s['edu']} child={s['child']}"
        )
        print(f"  prompt: {prompt!r}")
        print(f"  text:   {(prompt + ' ' + text)[:320]!r}\n")

    n_ok = sum(1 for r in results if r["ok"])
    print(f"{n_ok}/{len(results)} prompts passed (need {args.min_pass})")
    passed = n_ok >= args.min_pass
    if args.expect_fail:
        if passed:
            print("expected the baseline model to fail adult-hero scoring")
            return 1
        print("baseline correctly fails adult-hero scoring")
        return 0
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
