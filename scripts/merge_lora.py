#!/usr/bin/env python3
"""Merge a LoRA adapter into a new Safetensors candidate with numerical evidence."""

from __future__ import annotations

import argparse
import ctypes
import json
import math
import os
import shutil
import sys
import tempfile
from pathlib import Path

import torch
from peft import PeftModel
from transformers import AutoConfig, AutoModelForCausalLM

try:
    from transformers import AutoModelForImageTextToText
except ImportError:
    AutoModelForImageTextToText = None


DTYPES = {
    "float16": torch.float16,
    "bfloat16": torch.bfloat16,
    "float32": torch.float32,
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", type=Path, required=True)
    parser.add_argument("--adapter", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--dtype", choices=sorted(DTYPES), default="bfloat16")
    parser.add_argument("--device", default="cpu")
    parser.add_argument("--equivalence-inputs", type=Path)
    parser.add_argument("--equivalence-tolerance", type=float, default=1e-4)
    return parser.parse_args()


def model_loader(config: object):
    architectures = set(getattr(config, "architectures", None) or [])
    if (
        AutoModelForImageTextToText is not None
        and any("ConditionalGeneration" in name for name in architectures)
    ):
        return AutoModelForImageTextToText
    return AutoModelForCausalLM


def read_inputs(path: Path | None) -> list[list[int]]:
    if path is None:
        return [[1, 2, 3, 4]]
    payload = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(payload, list) or not payload:
        raise ValueError("equivalence inputs must be a non-empty JSON array")
    inputs: list[list[int]] = []
    for value in payload:
        if not isinstance(value, list) or not value or not all(isinstance(token, int) for token in value):
            raise ValueError("each equivalence input must be a non-empty integer array")
        inputs.append(value)
    return inputs


def adapter_delta(model: PeftModel) -> dict[str, float | int]:
    changed = 0
    unchanged = 0
    max_absolute = 0.0
    squared_norm = 0.0
    for module in model.modules():
        get_delta_weight = getattr(module, "get_delta_weight", None)
        active_adapters = getattr(module, "active_adapters", None)
        if get_delta_weight is None or not active_adapters:
            continue
        for adapter_name in active_adapters:
            delta = get_delta_weight(adapter_name).detach().float()
            if not torch.isfinite(delta).all():
                raise ValueError("adapter delta contains NaN or infinity")
            absolute = float(delta.abs().max().item()) if delta.numel() else 0.0
            norm = float(torch.linalg.vector_norm(delta).item()) if delta.numel() else 0.0
            if absolute > 0.0:
                changed += 1
                max_absolute = max(max_absolute, absolute)
                squared_norm += norm * norm
            else:
                unchanged += 1
    if changed == 0:
        raise ValueError("adapter merge is a no-op")
    return {
        "changed_tensor_count": changed,
        "unchanged_tensor_count": unchanged,
        "max_absolute_delta": max_absolute,
        "aggregate_norm_delta": math.sqrt(squared_norm),
    }


def logits(model: torch.nn.Module, inputs: list[list[int]], device: str) -> list[torch.Tensor]:
    outputs: list[torch.Tensor] = []
    model.eval()
    with torch.inference_mode():
        for tokens in inputs:
            input_ids = torch.tensor([tokens], dtype=torch.long, device=device)
            result = model(input_ids=input_ids, use_cache=False)
            value = result.logits.detach().float().cpu()
            if not torch.isfinite(value).all():
                raise ValueError("equivalence logits contain NaN or infinity")
            outputs.append(value)
    return outputs


def max_logit_delta(left: list[torch.Tensor], right: list[torch.Tensor]) -> float:
    if len(left) != len(right):
        raise ValueError("equivalence result count mismatch")
    maximum = 0.0
    for live, merged in zip(left, right, strict=True):
        if live.shape != merged.shape:
            raise ValueError("equivalence logit shape mismatch")
        maximum = max(maximum, float((live - merged).abs().max().item()))
    return maximum


def publish_directory_noclobber(source: Path, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    if sys.platform.startswith("linux"):
        libc = ctypes.CDLL(None, use_errno=True)
        renameat2 = getattr(libc, "renameat2", None)
        if renameat2 is None:
            raise OSError("renameat2 is required for no-clobber candidate publication")
        result = renameat2(
            -100,
            os.fsencode(source),
            -100,
            os.fsencode(destination),
            1,
        )
        if result != 0:
            error = ctypes.get_errno()
            raise OSError(error, os.strerror(error), destination)
        return
    if destination.exists():
        raise FileExistsError(destination)
    source.rename(destination)


def copy_substrate_assets(base: Path, candidate: Path) -> None:
    for source in base.iterdir():
        if not source.is_file():
            continue
        name = source.name
        if name == "model.safetensors.index.json" or name.endswith(".safetensors"):
            continue
        shutil.copy2(source, candidate / name)


def main() -> int:
    args = parse_args()
    if args.output.exists():
        raise FileExistsError(f"candidate output already exists: {args.output}")
    if args.report.exists():
        raise FileExistsError(f"merge report already exists: {args.report}")
    dtype = DTYPES[args.dtype]
    config = AutoConfig.from_pretrained(args.base, trust_remote_code=False)
    loader = model_loader(config)
    base = loader.from_pretrained(
        args.base,
        config=config,
        dtype=dtype,
        device_map=args.device,
        low_cpu_mem_usage=True,
        trust_remote_code=False,
    )
    model = PeftModel.from_pretrained(base, args.adapter, is_trainable=False)
    effective_dtype = str(next(model.parameters()).dtype).removeprefix("torch.")
    delta = adapter_delta(model)
    inputs = read_inputs(args.equivalence_inputs)
    live_logits = logits(model, inputs, args.device)
    merged = model.merge_and_unload(safe_merge=True)
    merged_logits = logits(merged, inputs, args.device)
    equivalence_delta = max_logit_delta(live_logits, merged_logits)
    if equivalence_delta > args.equivalence_tolerance:
        raise ValueError(
            f"adapter/merged equivalence failed: {equivalence_delta} > {args.equivalence_tolerance}"
        )

    temporary_root = Path(
        tempfile.mkdtemp(prefix=f".{args.output.name}.tmp-", dir=args.output.parent)
    )
    try:
        merged.save_pretrained(temporary_root, safe_serialization=True)
        copy_substrate_assets(args.base, temporary_root)
        publish_directory_noclobber(temporary_root, args.output)
    except BaseException:
        shutil.rmtree(temporary_root, ignore_errors=True)
        raise

    report = {
        "schema_version": 1,
        "base": str(args.base.resolve()),
        "adapter": str(args.adapter.resolve()),
        "candidate_path": str(args.output.resolve()),
        "requested_dtype": args.dtype,
        "effective_load_dtype": effective_dtype,
        "merge_dtype": effective_dtype,
        "output_dtype": effective_dtype,
        "delta": delta,
        "equivalence_max_absolute_delta": equivalence_delta,
        "equivalence_tolerance": args.equivalence_tolerance,
        "torch_version": torch.__version__,
    }
    try:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        descriptor = os.open(args.report, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
            json.dump(report, handle, indent=2, sort_keys=True)
            handle.write("\n")
    except BaseException:
        shutil.rmtree(args.output, ignore_errors=True)
        raise
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
