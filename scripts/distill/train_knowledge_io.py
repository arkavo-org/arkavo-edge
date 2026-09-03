#!/usr/bin/env python3
"""Checkpoint I/O for train_knowledge.py: load the base model, drop the
unused VL/MTP submodules, and push finished checkpoints to the Hub.

Split out of train_knowledge.py because this is a genuinely distinct
responsibility from the training loop itself (model/disk/network I/O vs.
the step-by-step optimization), not to pad a line count.
"""

from __future__ import annotations

import gc
from pathlib import Path

import torch
from huggingface_hub import HfApi
from transformers import AutoModelForCausalLM, AutoModelForImageTextToText


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
