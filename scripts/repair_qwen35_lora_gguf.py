#!/usr/bin/env python3
"""Repair Qwen3.5 linear-attention LoRA GGUF tensors after conversion.

The current llama.cpp LoRA converter can mis-handle Qwen3.5 linear-attention
``out_proj`` LoRA-A tensors when it applies the grouped-to-tiled V-head reorder
to the synthetic LoRA tensor wrapper. This script rewrites the affected GGUF
adapter tensors directly from the original PEFT safetensors using the same
column permutation expected by llama.cpp.
"""

from __future__ import annotations

import argparse
import json
import struct
from pathlib import Path
from typing import Any

import numpy as np
from gguf import GGUFReader


def parse_args() -> argparse.Namespace:
    """Parse repair inputs."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--adapter-dir", type=Path, required=True)
    parser.add_argument("--lora-gguf", type=Path, required=True)
    parser.add_argument("--base-config", type=Path, required=True)
    return parser.parse_args()


def load_text_config(path: Path) -> dict[str, Any]:
    """Load Qwen text config from a base model config.json."""
    config = json.loads(path.read_text())
    text_config = config.get("text_config", config)
    return text_config


def load_safetensors_header(path: Path) -> tuple[int, dict[str, Any]]:
    """Read safetensors data offset and JSON header."""
    with path.open("rb") as handle:
        header_len = struct.unpack("<Q", handle.read(8))[0]
        header = json.loads(handle.read(header_len))
    return 8 + header_len, header


def read_f32_tensor(path: Path, data_base: int, header: dict[str, Any], name: str) -> np.ndarray:
    """Read one F32 safetensors tensor."""
    meta = header[name]
    if meta["dtype"] != "F32":
        raise ValueError(f"{name}: expected F32, found {meta['dtype']}")
    start, end = meta["data_offsets"]
    with path.open("rb") as handle:
        handle.seek(data_base + start)
        data = handle.read(end - start)
    return np.frombuffer(data, dtype="<f4").reshape(meta["shape"])


def qwen35_v_head_column_permutation(text_config: dict[str, Any]) -> np.ndarray:
    """Return the grouped-to-tiled column permutation for Qwen3.5 V heads."""
    num_k_heads = int(text_config["linear_num_key_heads"])
    num_v_heads = int(text_config["linear_num_value_heads"])
    head_v_dim = int(text_config["linear_value_head_dim"])
    if num_v_heads % num_k_heads != 0:
        raise ValueError("linear_num_value_heads must be divisible by linear_num_key_heads")
    num_v_per_k = num_v_heads // num_k_heads
    values = np.arange(num_v_heads * head_v_dim, dtype=np.int64).reshape(1, -1)
    reordered = (
        values.reshape(1, num_k_heads, num_v_per_k, head_v_dim)
        .transpose(0, 2, 1, 3)
        .reshape(1, -1)
        .squeeze(0)
    )
    return reordered


def repair(adapter_dir: Path, lora_gguf: Path, base_config: Path) -> int:
    """Repair all Qwen3.5 ssm_out LoRA-A tensors in a GGUF adapter."""
    safetensors_path = adapter_dir / "adapter_model.safetensors"
    if not safetensors_path.is_file():
        raise FileNotFoundError(safetensors_path)
    if not lora_gguf.is_file():
        raise FileNotFoundError(lora_gguf)

    text_config = load_text_config(base_config)
    col_perm = qwen35_v_head_column_permutation(text_config)
    data_base, header = load_safetensors_header(safetensors_path)
    header_names = set(header) - {"__metadata__"}

    reader = GGUFReader(lora_gguf, mode="r+")
    repaired = 0
    for tensor in reader.tensors:
        name = tensor.name
        if not name.endswith(".ssm_out.weight.lora_a"):
            continue
        parts = name.split(".")
        if len(parts) < 4 or parts[0] != "blk":
            continue
        layer = int(parts[1])
        safetensors_name = (
            f"base_model.model.model.layers.{layer}."
            "linear_attn.out_proj.lora_A.weight"
        )
        if safetensors_name not in header_names:
            continue
        source = read_f32_tensor(safetensors_path, data_base, header, safetensors_name)
        expected = source[:, col_perm].astype(tensor.data.dtype, copy=False)
        if expected.shape != tensor.data.shape:
            raise ValueError(
                f"{name}: expected repair shape {expected.shape}, "
                f"GGUF shape is {tensor.data.shape}"
            )
        tensor.data[...] = expected
        repaired += 1

    # Flush all memmaps exposed by the reader.
    for tensor in reader.tensors:
        flush = getattr(tensor.data, "flush", None)
        if flush is not None:
            flush()

    if repaired == 0:
        raise RuntimeError("no Qwen3.5 ssm_out LoRA-A tensors were repaired")
    return repaired


def main() -> int:
    """Run the repair command."""
    args = parse_args()
    repaired = repair(args.adapter_dir, args.lora_gguf, args.base_config)
    print(f"repaired_qwen35_lora_gguf_tensors={repaired}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
