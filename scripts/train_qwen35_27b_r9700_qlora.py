#!/usr/bin/env python3
"""Train a Qwen3.5 27B adapter with ROCm QLoRA.

This script is intentionally container-friendly. It expects the model shards and
dataset to be mounted into the runtime and writes only PEFT adapter artifacts.
"""

from __future__ import annotations

import argparse
import hashlib
import inspect
import json
import os
from pathlib import Path
from typing import Any


DEFAULT_MODEL_ID = "/workspace/training-source/safe_tensors"
DEFAULT_DATA_PATH = "/workspace/training-source/data/train.jsonl"
DEFAULT_OUT_DIR = "/workspace/lmml/outputs/qlora/qwen35-27b-r9700-qlora"
DEFAULT_TARGET_MODULES = (
    "q_proj,k_proj,v_proj,o_proj,"
    "in_proj_a,in_proj_b,in_proj_qkv,in_proj_z,out_proj"
)


def env(name: str, default: str) -> str:
    """Read a string environment override."""
    value = os.environ.get(name)
    if value is None or value == "":
        return default
    return value


def env_int(name: str, default: int) -> int:
    """Read an integer environment override."""
    return int(env(name, str(default)))


def env_float(name: str, default: float) -> float:
    """Read a floating point environment override."""
    return float(env(name, str(default)))


def parse_args() -> argparse.Namespace:
    """Parse CLI arguments, with environment-variable defaults."""
    parser = argparse.ArgumentParser(
        description="Train a Qwen3.5 27B LoRA adapter with ROCm QLoRA.",
    )
    parser.add_argument("--model-id", default=env("MODEL_ID", DEFAULT_MODEL_ID))
    parser.add_argument("--data-path", default=env("DATA_PATH", DEFAULT_DATA_PATH))
    parser.add_argument("--out-dir", default=env("OUT_DIR", DEFAULT_OUT_DIR))
    parser.add_argument("--seq-len", type=int, default=env_int("SEQ_LEN", 512))
    parser.add_argument("--lora-r", type=int, default=env_int("LORA_R", 8))
    parser.add_argument("--lora-alpha", type=int, default=env_int("LORA_ALPHA", 16))
    parser.add_argument("--lora-dropout", type=float, default=env_float("LORA_DROPOUT", 0.05))
    parser.add_argument("--grad-accum", type=int, default=env_int("GRAD_ACCUM", 8))
    parser.add_argument("--epochs", type=float, default=env_float("NUM_EPOCHS", 1.0))
    parser.add_argument("--max-steps", type=int, default=env_int("MAX_STEPS", -1))
    parser.add_argument("--max-samples", type=int, default=env_int("MAX_SAMPLES", 0))
    parser.add_argument("--learning-rate", type=float, default=env_float("LR", 1e-5))
    parser.add_argument("--optim", default=env("OPTIM", "adamw_torch"))
    parser.add_argument(
        "--empty-cache-steps",
        type=int,
        default=env_int("EMPTY_CACHE_STEPS", 0),
        help="Call torch.cuda.empty_cache every N optimizer steps; 0 disables it.",
    )
    parser.add_argument(
        "--log-memory-steps",
        type=int,
        default=env_int("LOG_MEMORY_STEPS", 1),
        help="Log ROCm/PyTorch memory counters every N optimizer steps; 0 disables it.",
    )
    parser.add_argument(
        "--save-strategy",
        choices=["no", "steps", "epoch"],
        default=env("SAVE_STRATEGY", "steps"),
        help="Checkpoint strategy. Final adapter is always saved at the end.",
    )
    parser.add_argument("--save-steps", type=int, default=env_int("SAVE_STEPS", 5))
    parser.add_argument("--save-total-limit", type=int, default=env_int("SAVE_TOTAL_LIMIT", 2))
    parser.add_argument(
        "--trace-batch-rows",
        action="store_true",
        default=env("TRACE_BATCH_ROWS", "0") == "1",
        help="Print source row IDs for each microbatch before model execution.",
    )
    parser.add_argument(
        "--suspect-rows",
        default=env("SUSPECT_ROWS", ""),
        help="Comma-separated zero-based row IDs to summarize after tokenization.",
    )
    parser.add_argument(
        "--only-source-lines",
        default=env("ONLY_SOURCE_LINES", ""),
        help="Comma-separated JSONL source lines to load for direct replay.",
    )
    parser.add_argument(
        "--pad-to-multiple-of",
        type=int,
        default=env_int("PAD_TO_MULTIPLE_OF", 0),
        help="Pad dynamic batches to this token multiple; 0 disables alignment.",
    )
    parser.add_argument(
        "--shuffle-data",
        action="store_true",
        default=env("SHUFFLE_DATA", "1") == "1",
        help="Use Trainer's default random sampler. Set SHUFFLE_DATA=0 for replay order.",
    )
    parser.add_argument(
        "--trace-backward-modules",
        action="store_true",
        default=env("TRACE_BACKWARD_MODULES", "0") == "1",
        help="Trace selected module backward hooks during one-row diagnostics.",
    )
    parser.add_argument(
        "--backward-trace-markers",
        default=env("BACKWARD_TRACE_MARKERS", "Linear4bit,GatedDeltaNet,DeltaNet"),
        help="Comma-separated module class/name markers for backward tracing.",
    )
    parser.add_argument(
        "--trace-backward-prefix",
        default=env("TRACE_BACKWARD_PREFIX", ""),
        help="Exact module subtree prefix for focused backward tracing.",
    )
    parser.add_argument(
        "--lora-exclude-modules",
        default=env("LORA_EXCLUDE_MODULES", ""),
        help="Comma-separated module names or regex:... to exclude from LoRA.",
    )
    parser.add_argument(
        "--lora-autocast-adapter-dtype",
        action="store_true",
        default=env("LORA_AUTOCAST_ADAPTER_DTYPE", "1").lower()
        not in {"0", "false", "no"},
        help="Allow PEFT to autocast adapter weights to FP32 where supported.",
    )
    parser.add_argument(
        "--rocm-blas-backend",
        choices=["", "rocblas", "hipblaslt"],
        default=env("ROCM_BLAS_BACKEND", ""),
        help="Request a ROCm GEMM backend through PyTorch's BLAS selector.",
    )
    parser.add_argument(
        "--dataloader-num-workers",
        type=int,
        default=env_int("DATALOADER_NUM_WORKERS", 0),
    )
    parser.add_argument(
        "--dataloader-pin-memory",
        action="store_true",
        default=env("DATALOADER_PIN_MEMORY", "0") == "1",
    )
    parser.add_argument(
        "--dataloader-persistent-workers",
        action="store_true",
        default=env("DATALOADER_PERSISTENT_WORKERS", "0") == "1",
    )
    parser.add_argument(
        "--attn-implementation",
        choices=["default", "eager", "sdpa", "flash_attention_2"],
        default=env("ATTN_IMPLEMENTATION", "eager"),
        help="Transformers attention backend. Use eager for conservative ROCm smoke runs.",
    )
    parser.add_argument(
        "--force-math-sdp",
        action="store_true",
        default=env("FORCE_MATH_SDP", "1") == "1",
        help="Disable flash/mem-efficient SDP and force math SDP when available.",
    )
    parser.add_argument(
        "--compute-dtype",
        choices=["auto", "bf16", "fp16"],
        default=env("COMPUTE_DTYPE", "auto"),
    )
    parser.add_argument(
        "--target-modules",
        default=env("TARGET_MODULES", DEFAULT_TARGET_MODULES),
        help="Comma-separated PEFT target module suffixes.",
    )
    parser.add_argument(
        "--prepare-only",
        action="store_true",
        default=env("PREPARE_ONLY", "0") == "1",
        help="Load tokenizer and dataset, then exit before loading the model.",
    )
    return parser.parse_args()


def content_text(value: Any) -> str:
    """Render JSON-like message content into stable training text."""
    if isinstance(value, str):
        return value
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"))


def load_training_rows(
    path: Path,
    max_samples: int,
    only_source_lines: set[int],
) -> list[dict[str, Any]]:
    """Load chat JSONL rows into system/user/assistant triples."""
    rows: list[dict[str, Any]] = []
    with path.open("r", encoding="utf-8") as handle:
        for line_num, line in enumerate(handle, 1):
            if only_source_lines and line_num not in only_source_lines:
                continue
            line = line.strip()
            if not line:
                continue
            try:
                example = json.loads(line)
            except json.JSONDecodeError as error:
                print(f"[dataset] skip line {line_num}: {error}")
                continue
            if not isinstance(example, dict):
                print(f"[dataset] skip line {line_num}: expected object")
                continue

            row = unpack_messages(example)
            if row is not None:
                row["__source_line__"] = line_num
                rows.append(row)
            if max_samples > 0 and len(rows) >= max_samples:
                break
            if line_num % 500 == 0:
                print(f"[dataset] scanned {line_num} lines; loaded {len(rows)} rows")
    if not rows:
        if only_source_lines:
            raise ValueError(
                f"no rows matched ONLY_SOURCE_LINES={sorted(only_source_lines)}"
            )
        raise ValueError(f"no trainable rows found in {path}")
    return rows


def unpack_messages(example: dict[str, Any]) -> dict[str, str] | None:
    """Normalize supported row shapes into one chat example."""
    if isinstance(example.get("messages"), list):
        system = ""
        user = ""
        assistant = ""
        for message in example["messages"]:
            if not isinstance(message, dict):
                continue
            role = message.get("role")
            content = content_text(message.get("content", ""))
            if role == "system":
                system = content
            elif role == "user":
                user = content
            elif role == "assistant":
                assistant = content
        if user or assistant:
            return {"system": system, "user": user, "assistant": assistant}

    if "human" in example and "gpt" in example:
        return {
            "system": content_text(example.get("system", "")),
            "user": content_text(example["human"]),
            "assistant": content_text(example["gpt"]),
        }
    return None


def apply_chat_template(
    tokenizer: Any,
    messages: list[dict[str, str]],
    *,
    add_generation_prompt: bool = False,
) -> str:
    """Render messages through the model tokenizer or a ChatML fallback."""
    try:
        return tokenizer.apply_chat_template(
            messages,
            tokenize=False,
            add_generation_prompt=add_generation_prompt,
        )
    except Exception:
        rendered = "".join(
            f"<|im_start|>{message['role']}\n{message['content']}<|im_end|>\n"
            for message in messages
        )
        if add_generation_prompt:
            rendered += "<|im_start|>assistant\n"
        return rendered


def build_preprocess_function(tokenizer: Any, seq_len: int):
    """Build a tokenizer mapping function with assistant-only loss masking."""

    def preprocess(example: dict[str, Any]) -> dict[str, list[int]]:
        prompt_messages = []
        if example["system"].strip():
            prompt_messages.append({"role": "system", "content": example["system"]})
        prompt_messages.append({"role": "user", "content": example["user"]})
        full_messages = prompt_messages + [
            {"role": "assistant", "content": example["assistant"]}
        ]

        full_text = apply_chat_template(tokenizer, full_messages)
        encoded = tokenizer(
            full_text,
            truncation=True,
            max_length=seq_len,
            add_special_tokens=False,
        )
        tokenized = {
            key: list(value)
            for key, value in encoded.items()
        }
        input_ids = tokenized["input_ids"]
        if "attention_mask" not in tokenized:
            tokenized["attention_mask"] = [1] * len(input_ids)

        assistant_header_ids = tokenizer(
            "<|im_start|>assistant\n",
            add_special_tokens=False,
        )["input_ids"]
        assistant_start = None
        header_len = len(assistant_header_ids)
        for index in range(len(input_ids) - header_len + 1):
            if input_ids[index : index + header_len] == assistant_header_ids:
                assistant_start = index + header_len
                break

        if assistant_start is None:
            prompt_text = apply_chat_template(
                tokenizer,
                prompt_messages,
                add_generation_prompt=True,
            )
            prompt_ids = tokenizer(
                prompt_text,
                truncation=True,
                max_length=seq_len,
                add_special_tokens=False,
            )["input_ids"]
            assistant_start = min(len(prompt_ids), len(input_ids))

        labels = [-100] * assistant_start + input_ids[assistant_start:]
        tokenized["labels"] = labels
        return tokenized

    return preprocess


class TokenizedDataset:
    """Small in-memory dataset for tokenized training examples."""

    def __init__(self, rows: list[dict[str, Any]]) -> None:
        self.rows = rows

    def __len__(self) -> int:
        return len(self.rows)

    def __getitem__(self, index: int) -> dict[str, Any]:
        return self.rows[index]


def parse_row_indices(value: str) -> list[int]:
    """Parse comma-separated zero-based row IDs."""
    indices = []
    for part in value.split(","):
        part = part.strip()
        if not part:
            continue
        index = int(part)
        if index < 0:
            raise ValueError(f"suspect row index must be non-negative: {index}")
        indices.append(index)
    return indices


def parse_source_lines(value: str) -> set[int]:
    """Parse comma-separated one-based JSONL source line numbers."""
    source_lines = set()
    for part in value.split(","):
        part = part.strip()
        if not part:
            continue
        line_num = int(part)
        if line_num <= 0:
            raise ValueError(f"source line must be positive: {line_num}")
        source_lines.add(line_num)
    return source_lines


def validate_tokenized_row(
    row: dict[str, Any],
    index: int,
    seq_len: int,
    vocab_size: int,
) -> None:
    """Reject malformed tokenized rows before they reach ROCm kernels."""
    input_ids = row.get("input_ids")
    attention_mask = row.get("attention_mask")
    labels = row.get("labels")

    if not isinstance(input_ids, list) or not input_ids:
        raise ValueError(f"row {index}: empty input_ids")
    if not isinstance(attention_mask, list):
        raise ValueError(f"row {index}: missing attention_mask")
    if not isinstance(labels, list):
        raise ValueError(f"row {index}: missing labels")
    if len(input_ids) != len(attention_mask):
        raise ValueError(
            f"row {index}: input_ids={len(input_ids)} "
            f"attention_mask={len(attention_mask)}"
        )
    if len(input_ids) != len(labels):
        raise ValueError(
            f"row {index}: input_ids={len(input_ids)} labels={len(labels)}"
        )
    if len(input_ids) > seq_len:
        raise ValueError(f"row {index}: length {len(input_ids)} exceeds {seq_len}")
    if not any(label != -100 for label in labels):
        raise ValueError(f"row {index}: all labels are masked")
    if any(mask not in (0, 1) for mask in attention_mask):
        raise ValueError(f"row {index}: invalid attention-mask value")
    invalid_tokens = [
        token_id
        for token_id in input_ids
        if not isinstance(token_id, int) or token_id < 0 or token_id >= vocab_size
    ]
    if invalid_tokens:
        raise ValueError(
            f"row {index}: invalid input token IDs: {invalid_tokens[:10]}"
        )
    invalid_labels = [
        label
        for label in labels
        if label != -100
        and (not isinstance(label, int) or label < 0 or label >= vocab_size)
    ]
    if invalid_labels:
        raise ValueError(
            f"row {index}: invalid label IDs: {invalid_labels[:10]}"
        )


def print_dataset_shape_summary(rows: list[dict[str, Any]]) -> None:
    """Print compact token and target-length distribution diagnostics."""
    lengths = [int(sum(row["attention_mask"])) for row in rows]
    target_lengths = [
        sum(1 for label in row["labels"] if label != -100)
        for row in rows
    ]
    print(
        "[dataset-shapes] "
        f"rows={len(lengths)} "
        f"min={min(lengths)} "
        f"max={max(lengths)} "
        f"mean={sum(lengths) / len(lengths):.2f} "
        f"target_min={min(target_lengths)} "
        f"target_max={max(target_lengths)} "
        f"target_mean={sum(target_lengths) / len(target_lengths):.2f}",
        flush=True,
    )


def trace_values(value: Any) -> list[Any]:
    """Normalize diagnostic row IDs from Python lists or tensors."""
    if value is None:
        return []
    if hasattr(value, "detach"):
        return value.detach().cpu().tolist()
    return list(value)


def parse_markers(value: str) -> list[str]:
    """Parse comma-separated module class/name markers."""
    return [marker.strip() for marker in value.split(",") if marker.strip()]


def parse_lora_exclude_modules(value: str) -> str | list[str] | None:
    """Parse PEFT LoRA exclude_modules from environment-friendly text."""
    value = value.strip()
    if not value:
        return None
    if value.startswith("regex:"):
        return value[len("regex:") :].strip()
    parts = [part.strip() for part in value.split(",") if part.strip()]
    if len(parts) == 1 and any(char in parts[0] for char in "*^$\\[]()|+?"):
        return parts[0]
    return parts


def exact_lora_exclusion_prefixes(value: str) -> list[str]:
    """Return concrete module prefixes that should have no LoRA children."""
    prefixes = []
    for part in value.split(","):
        part = part.strip()
        if not part or part.startswith("regex:"):
            continue
        if any(char in part for char in "*^$\\[]()|+?"):
            continue
        prefixes.append(part)
        if not part.startswith("base_model."):
            prefixes.append(f"base_model.model.model.{part}")
    seen = set()
    ordered = []
    for prefix in prefixes:
        if prefix in seen:
            continue
        seen.add(prefix)
        ordered.append(prefix)
    return ordered


def trace_prefix_candidates(prefix: str) -> list[str]:
    """Return exact-subtree trace prefixes, including PEFT's base_model prefix."""
    prefix = prefix.strip()
    if not prefix:
        return []
    candidates = [prefix]
    if not prefix.startswith("base_model."):
        candidates.insert(0, f"base_model.model.model.{prefix}")
    seen = set()
    ordered = []
    for candidate in candidates:
        if candidate in seen:
            continue
        seen.add(candidate)
        ordered.append(candidate)
    return ordered


def shape_summary(values: Any) -> list[Any]:
    """Summarize hook tensor shapes without retaining tensors."""
    shapes = []
    for value in values or ():
        if hasattr(value, "shape"):
            shapes.append(tuple(value.shape))
        else:
            shapes.append(type(value).__name__)
    return shapes


def configure_rocm_blas_backend(torch_module: Any, backend: str) -> None:
    """Select rocBLAS or hipBLASLt through PyTorch's CUDA-compatible API."""
    backend = backend.strip().lower()
    if not backend:
        return
    if not hasattr(torch_module.backends, "cuda") or not hasattr(
        torch_module.backends.cuda,
        "preferred_blas_library",
    ):
        raise RuntimeError(
            "This PyTorch build does not expose preferred_blas_library()."
        )
    backend_map = {
        "rocblas": "cublas",
        "hipblaslt": "cublaslt",
    }
    if backend not in backend_map:
        raise ValueError("ROCM_BLAS_BACKEND must be 'rocblas' or 'hipblaslt'.")
    requested = backend_map[backend]
    torch_module.backends.cuda.preferred_blas_library(requested)
    selected = torch_module.backends.cuda.preferred_blas_library()
    hipblaslt_env = os.getenv("ROCBLAS_USE_HIPBLASLT", "")
    print(
        "[runtime] "
        f"requested_rocm_blas={backend} "
        f"pytorch_blas_backend={selected} "
        f"ROCBLAS_USE_HIPBLASLT={hipblaslt_env}",
        flush=True,
    )
    if backend == "rocblas" and hipblaslt_env != "0":
        raise RuntimeError(
            "R9700 QLoRA safety configuration requires "
            "ROCBLAS_USE_HIPBLASLT=0 when ROCM_BLAS_BACKEND=rocblas."
        )


def print_lora_dtype_probe(model: Any, module_name: str) -> None:
    """Print adapter dtype for a known LoRA probe module when present."""
    try:
        probe = model.get_submodule(module_name)
    except AttributeError:
        print(f"[lora-dtype] module={module_name} missing=True", flush=True)
        return
    weight = getattr(probe, "weight", None)
    if weight is None:
        print(f"[lora-dtype] module={module_name} weight_missing=True", flush=True)
        return
    print(
        "[lora-dtype] "
        f"module={module_name} "
        f"weight_dtype={weight.dtype} "
        f"weight_shape={tuple(weight.shape)}",
        flush=True,
    )


def print_tokenized_row_summary(row_index: int, item: dict[str, Any]) -> None:
    """Print tensor-shape diagnostics for one tokenized row."""
    input_ids = item["input_ids"]
    attention_mask = item["attention_mask"]
    labels = item["labels"]
    active_length = int(sum(attention_mask))
    target_positions = [
        position
        for position, label in enumerate(labels)
        if label != -100
    ]
    print(
        "[suspect-row] "
        f"row={row_index} "
        f"source_line={item.get('__source_line__')} "
        f"stored_length={len(input_ids)} "
        f"active_length={active_length} "
        f"target_count={len(target_positions)} "
        f"target_first={target_positions[0] if target_positions else None} "
        f"target_last={target_positions[-1] if target_positions else None} "
        f"pad_count={len(input_ids) - active_length}",
        flush=True,
    )


def print_suspect_row_summaries(dataset: TokenizedDataset, row_indices: list[int]) -> None:
    """Print tensor-shape diagnostics for selected dataset indices."""
    for row_index in row_indices:
        if row_index >= len(dataset):
            print(
                f"[suspect-row] row={row_index} skipped: "
                f"dataset_rows={len(dataset)}",
                flush=True,
            )
            continue
        print_tokenized_row_summary(row_index, dataset[row_index])


def print_source_line_summaries(
    dataset: TokenizedDataset,
    source_lines: set[int],
) -> None:
    """Print diagnostics for selected source-line rows after filtering."""
    matched_lines = set()
    for item in dataset.rows:
        source_line = int(item.get("__source_line__", 0))
        if source_line not in source_lines:
            continue
        matched_lines.add(source_line)
        print_tokenized_row_summary(int(item.get("__row_id__", -1)), item)
    missing_lines = sorted(source_lines - matched_lines)
    if missing_lines:
        print(f"[suspect-row] missing_source_lines={missing_lines}", flush=True)


def tokenize_rows(
    rows: list[dict[str, Any]],
    tokenizer: Any,
    seq_len: int,
) -> TokenizedDataset:
    """Tokenize rows without Hugging Face datasets/pyarrow fingerprinting."""
    preprocess = build_preprocess_function(tokenizer, seq_len)
    vocab_size = len(tokenizer)
    tokenized_rows: list[dict[str, Any]] = []
    for index, row in enumerate(rows, 1):
        tokenized = preprocess(row)
        tokenized["__source_line__"] = int(row.get("__source_line__", index))
        tokenized["__row_id__"] = tokenized["__source_line__"] - 1
        validate_tokenized_row(tokenized, index, seq_len, vocab_size)
        tokenized_rows.append(tokenized)
        if index % 500 == 0:
            print(f"[dataset] tokenized {index} rows")
    print_dataset_shape_summary(tokenized_rows)
    return TokenizedDataset(tokenized_rows)


def sha256(path: Path) -> str:
    """Return a SHA256 digest for a local file."""
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def write_manifest(
    out_dir: Path,
    args: argparse.Namespace,
    rows: int,
    torch_version: str,
    transformers_version: str,
    peft_version: str,
) -> None:
    """Write a small reproducibility manifest beside the adapter."""
    manifest = {
        "model_id": args.model_id,
        "data_path": args.data_path,
        "data_sha256": sha256(Path(args.data_path)),
        "rows": rows,
        "seq_len": args.seq_len,
        "lora_r": args.lora_r,
        "lora_alpha": args.lora_alpha,
        "lora_dropout": args.lora_dropout,
        "grad_accum": args.grad_accum,
        "epochs": args.epochs,
        "max_steps": args.max_steps,
        "learning_rate": args.learning_rate,
        "optim": args.optim,
        "empty_cache_steps": args.empty_cache_steps,
        "log_memory_steps": args.log_memory_steps,
        "save_strategy": args.save_strategy,
        "save_steps": args.save_steps,
        "save_total_limit": args.save_total_limit,
        "trace_batch_rows": args.trace_batch_rows,
        "suspect_rows": parse_row_indices(args.suspect_rows),
        "only_source_lines": sorted(parse_source_lines(args.only_source_lines)),
        "pad_to_multiple_of": args.pad_to_multiple_of,
        "shuffle_data": args.shuffle_data,
        "trace_backward_modules": args.trace_backward_modules,
        "backward_trace_markers": parse_markers(args.backward_trace_markers),
        "trace_backward_prefix": args.trace_backward_prefix,
        "lora_exclude_modules": args.lora_exclude_modules,
        "lora_autocast_adapter_dtype": args.lora_autocast_adapter_dtype,
        "rocm_blas_backend": args.rocm_blas_backend,
        "rocblas_use_hipblaslt": os.getenv("ROCBLAS_USE_HIPBLASLT", ""),
        "dataloader_num_workers": args.dataloader_num_workers,
        "dataloader_pin_memory": args.dataloader_pin_memory,
        "dataloader_persistent_workers": args.dataloader_persistent_workers,
        "attn_implementation": args.attn_implementation,
        "force_math_sdp": args.force_math_sdp,
        "compute_dtype": args.compute_dtype,
        "target_modules": [
            module.strip()
            for module in args.target_modules.split(",")
            if module.strip()
        ],
        "torch": torch_version,
        "transformers": transformers_version,
        "peft": peft_version,
    }
    (out_dir / "lmml-qlora-manifest.json").write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def main() -> int:
    """Run the QLoRA adapter training flow."""
    args = parse_args()
    model_id = Path(args.model_id)
    data_path = Path(args.data_path)
    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)

    import peft
    import torch
    import transformers
    from peft import LoraConfig, get_peft_model, prepare_model_for_kbit_training
    from transformers import (
        AutoModelForCausalLM,
        AutoTokenizer,
        BitsAndBytesConfig,
        DataCollatorForSeq2Seq,
        Trainer,
        TrainingArguments,
        set_seed,
    )

    configure_rocm_blas_backend(torch, args.rocm_blas_backend)
    set_seed(42)
    print(f"[config] MODEL_ID={model_id}")
    print(f"[config] DATA_PATH={data_path}")
    print(f"[config] OUT_DIR={out_dir}")
    print(f"[config] SEQ_LEN={args.seq_len}")
    print(f"[config] LORA_R={args.lora_r}")
    print(f"[config] GRAD_ACCUM={args.grad_accum}")
    print(f"[config] MAX_STEPS={args.max_steps}")
    if args.max_steps > 0:
        print(f"[config] EFFECTIVE_SMOKE_EXAMPLES={args.max_steps * args.grad_accum}")
    print(f"[config] OPTIM={args.optim}")
    print(f"[config] EMPTY_CACHE_STEPS={args.empty_cache_steps}")
    print(f"[config] LOG_MEMORY_STEPS={args.log_memory_steps}")
    print(f"[config] SAVE_STRATEGY={args.save_strategy}")
    print(f"[config] SAVE_STEPS={args.save_steps}")
    print(f"[config] SAVE_TOTAL_LIMIT={args.save_total_limit}")
    print(f"[config] TRACE_BATCH_ROWS={args.trace_batch_rows}")
    print(f"[config] SUSPECT_ROWS={args.suspect_rows}")
    print(f"[config] ONLY_SOURCE_LINES={args.only_source_lines}")
    print(f"[config] PAD_TO_MULTIPLE_OF={args.pad_to_multiple_of}")
    print(f"[config] SHUFFLE_DATA={args.shuffle_data}")
    print(f"[config] TRACE_BACKWARD_MODULES={args.trace_backward_modules}")
    print(f"[config] BACKWARD_TRACE_MARKERS={args.backward_trace_markers}")
    print(f"[config] TRACE_BACKWARD_PREFIX={args.trace_backward_prefix}")
    print(f"[config] LORA_EXCLUDE_MODULES={args.lora_exclude_modules}")
    print(f"[config] LORA_AUTOCAST_ADAPTER_DTYPE={args.lora_autocast_adapter_dtype}")
    print(f"[config] ROCM_BLAS_BACKEND={args.rocm_blas_backend}")
    print(f"[config] ROCBLAS_USE_HIPBLASLT={os.getenv('ROCBLAS_USE_HIPBLASLT', '')}")
    print(f"[config] DATALOADER_NUM_WORKERS={args.dataloader_num_workers}")
    print(f"[config] DATALOADER_PIN_MEMORY={args.dataloader_pin_memory}")
    print(f"[config] DATALOADER_PERSISTENT_WORKERS={args.dataloader_persistent_workers}")
    print(f"[config] ATTN_IMPLEMENTATION={args.attn_implementation}")
    print(f"[config] FORCE_MATH_SDP={args.force_math_sdp}")
    print(f"[runtime] torch={torch.__version__}")
    print(f"[runtime] torch.version.hip={getattr(torch.version, 'hip', None)}")
    print(f"[runtime] torch.cuda.is_available={torch.cuda.is_available()}")
    if torch.cuda.is_available():
        print(f"[runtime] device={torch.cuda.get_device_name(0)}")
    if args.force_math_sdp and hasattr(torch.backends, "cuda"):
        torch.backends.cuda.enable_flash_sdp(False)
        torch.backends.cuda.enable_mem_efficient_sdp(False)
        torch.backends.cuda.enable_math_sdp(True)

    tokenizer = AutoTokenizer.from_pretrained(model_id, trust_remote_code=True)
    if tokenizer.pad_token is None:
        tokenizer.pad_token = tokenizer.eos_token
    tokenizer.padding_side = "right"

    only_source_lines = parse_source_lines(args.only_source_lines)
    rows = load_training_rows(data_path, args.max_samples, only_source_lines)
    dataset = tokenize_rows(rows, tokenizer, args.seq_len)
    suspect_rows = parse_row_indices(args.suspect_rows)
    if suspect_rows:
        print_suspect_row_summaries(dataset, suspect_rows)
    if only_source_lines:
        print_source_line_summaries(dataset, only_source_lines)
    print(f"[dataset] loaded rows={len(dataset)}")

    if args.prepare_only:
        write_manifest(
            out_dir,
            args,
            len(dataset),
            torch.__version__,
            transformers.__version__,
            peft.__version__,
        )
        print("[status] prepare-only complete")
        return 0

    if args.compute_dtype == "bf16":
        compute_dtype = torch.bfloat16
    elif args.compute_dtype == "fp16":
        compute_dtype = torch.float16
    elif torch.cuda.is_available() and torch.cuda.is_bf16_supported():
        compute_dtype = torch.bfloat16
    else:
        compute_dtype = torch.float16

    quantization = BitsAndBytesConfig(
        load_in_4bit=True,
        bnb_4bit_quant_type="nf4",
        bnb_4bit_compute_dtype=compute_dtype,
        bnb_4bit_use_double_quant=True,
    )
    model_kwargs = {
        "quantization_config": quantization,
        "device_map": "auto",
        "low_cpu_mem_usage": True,
        "trust_remote_code": True,
    }
    if args.attn_implementation != "default":
        model_kwargs["attn_implementation"] = args.attn_implementation
    model = AutoModelForCausalLM.from_pretrained(model_id, **model_kwargs)
    model.config.use_cache = False
    model = prepare_model_for_kbit_training(
        model,
        use_gradient_checkpointing=True,
        gradient_checkpointing_kwargs={"use_reentrant": False},
    )

    target_modules = [
        module.strip()
        for module in args.target_modules.split(",")
        if module.strip()
    ]
    lora_kwargs = {
        "r": args.lora_r,
        "lora_alpha": args.lora_alpha,
        "lora_dropout": args.lora_dropout,
        "bias": "none",
        "task_type": "CAUSAL_LM",
        "target_modules": target_modules,
    }
    exclude_modules = parse_lora_exclude_modules(args.lora_exclude_modules)
    if exclude_modules is not None:
        lora_params = set(inspect.signature(LoraConfig).parameters)
        if "exclude_modules" not in lora_params:
            raise RuntimeError(
                "This PEFT LoraConfig does not support exclude_modules; "
                "upgrade PEFT or remove LORA_EXCLUDE_MODULES."
            )
        lora_kwargs["exclude_modules"] = exclude_modules
        print(f"[lora-config] exclude_modules={exclude_modules}", flush=True)
    lora_config = LoraConfig(**lora_kwargs)
    get_peft_kwargs = {}
    get_peft_params = set(inspect.signature(get_peft_model).parameters)
    if "autocast_adapter_dtype" in get_peft_params:
        get_peft_kwargs["autocast_adapter_dtype"] = args.lora_autocast_adapter_dtype
    elif not args.lora_autocast_adapter_dtype:
        raise RuntimeError(
            "This PEFT get_peft_model() does not support "
            "autocast_adapter_dtype; upgrade PEFT or remove "
            "LORA_AUTOCAST_ADAPTER_DTYPE=0."
        )
    model = get_peft_model(model, lora_config, **get_peft_kwargs)
    if hasattr(model, "enable_input_require_grads"):
        model.enable_input_require_grads()
    print(
        "[lora-config] "
        f"autocast_adapter_dtype={args.lora_autocast_adapter_dtype}",
        flush=True,
    )
    print_lora_dtype_probe(
        model,
        "base_model.model.model.layers.4.linear_attn.out_proj.lora_A.default",
    )

    for prefix in exact_lora_exclusion_prefixes(args.lora_exclude_modules):
        matching_modules = [
            name
            for name, _ in model.named_modules()
            if name == prefix or name.startswith(prefix + ".")
        ]
        if not matching_modules:
            continue
        unexpected_adapters = [
            name
            for name in matching_modules
            if ".lora_A." in name or ".lora_B." in name
        ]
        if unexpected_adapters:
            raise RuntimeError(
                "Excluded module still contains LoRA adapters: "
                f"{unexpected_adapters}"
            )
        print(
            f"[lora-exclusion-check] prefix={prefix} adapter_present=False",
            flush=True,
        )

    backward_trace_handles = []
    if args.trace_backward_modules:
        markers = parse_markers(args.backward_trace_markers)

        def should_trace_module(module_name: str, class_name: str) -> bool:
            return any(
                marker in module_name or marker in class_name
                for marker in markers
            )

        for module_name, module in model.named_modules():
            class_name = module.__class__.__name__
            if not should_trace_module(module_name, class_name):
                continue

            def pre_hook(current_module, grad_output, *, name=module_name, cls=class_name):
                if torch.cuda.is_available():
                    torch.cuda.synchronize()
                print(
                    f"[backward-module-enter] name={name} class={cls}",
                    flush=True,
                )

            def post_hook(
                current_module,
                grad_input,
                grad_output,
                *,
                name=module_name,
                cls=class_name,
            ):
                if torch.cuda.is_available():
                    torch.cuda.synchronize()
                print(
                    f"[backward-module-exit] name={name} class={cls}",
                    flush=True,
                )

            backward_trace_handles.append(
                module.register_full_backward_pre_hook(pre_hook)
            )
            backward_trace_handles.append(
                module.register_full_backward_hook(post_hook)
            )
        print(
            f"[backward-trace] handles={len(backward_trace_handles)} "
            f"markers={markers}",
            flush=True,
        )

    trace_prefixes = trace_prefix_candidates(args.trace_backward_prefix)
    if trace_prefixes:
        matched_prefix = None
        for candidate in trace_prefixes:
            if any(
                name == candidate or name.startswith(candidate + ".")
                for name, _ in model.named_modules()
            ):
                matched_prefix = candidate
                break
        if matched_prefix is None:
            raise RuntimeError(
                "No modules matched TRACE_BACKWARD_PREFIX="
                f"{args.trace_backward_prefix!r}; candidates={trace_prefixes}"
            )
        for module_name, module in model.named_modules():
            if not (
                module_name == matched_prefix
                or module_name.startswith(matched_prefix + ".")
            ):
                continue
            class_name = module.__class__.__name__
            print(
                f"[subtree-hook] name={module_name} class={class_name}",
                flush=True,
            )

            def subtree_pre_hook(
                current_module,
                grad_output,
                *,
                name=module_name,
                cls=class_name,
            ):
                if torch.cuda.is_available():
                    torch.cuda.synchronize()
                print(
                    "[subtree-backward-enter] "
                    f"name={name} class={cls} "
                    f"grad_output={shape_summary(grad_output)}",
                    flush=True,
                )

            def subtree_post_hook(
                current_module,
                grad_input,
                grad_output,
                *,
                name=module_name,
                cls=class_name,
            ):
                if torch.cuda.is_available():
                    torch.cuda.synchronize()
                print(
                    "[subtree-backward-exit] "
                    f"name={name} class={cls} "
                    f"grad_input={shape_summary(grad_input)}",
                    flush=True,
                )

            backward_trace_handles.append(
                module.register_full_backward_pre_hook(subtree_pre_hook)
            )
            backward_trace_handles.append(
                module.register_full_backward_hook(subtree_post_hook)
            )
        print(
            f"[subtree-trace] installed_handles={len(backward_trace_handles)} "
            f"prefix={matched_prefix}",
            flush=True,
        )

    training_kwargs = {
        "output_dir": str(out_dir),
        "per_device_train_batch_size": 1,
        "gradient_accumulation_steps": args.grad_accum,
        "num_train_epochs": args.epochs,
        "max_steps": args.max_steps,
        "learning_rate": args.learning_rate,
        "warmup_steps": 10,
        "max_grad_norm": 1.0,
        "lr_scheduler_type": "cosine",
        "bf16": compute_dtype == torch.bfloat16,
        "fp16": compute_dtype == torch.float16,
        "gradient_checkpointing": True,
        "gradient_checkpointing_kwargs": {"use_reentrant": False},
        "logging_strategy": "steps",
        "logging_steps": 1,
        "save_strategy": args.save_strategy,
        "save_steps": args.save_steps,
        "save_total_limit": args.save_total_limit,
        "report_to": "none",
        "optim": args.optim,
        "remove_unused_columns": False,
        "dataloader_pin_memory": args.dataloader_pin_memory,
        "dataloader_num_workers": args.dataloader_num_workers,
        "dataloader_persistent_workers": args.dataloader_persistent_workers,
    }
    accepted_training_args = set(inspect.signature(TrainingArguments).parameters)
    training_args = TrainingArguments(
        **{
            key: value
            for key, value in training_kwargs.items()
            if key in accepted_training_args
        }
    )
    callbacks = []

    if args.log_memory_steps > 0 and torch.cuda.is_available():
        from transformers import TrainerCallback

        class RocmMemoryCallback(TrainerCallback):
            """Log accumulated ROCm/PyTorch memory high-water marks."""

            def on_step_end(self, args_, state, control, **kwargs):
                if state.global_step % args.log_memory_steps != 0:
                    return control
                free_bytes, total_bytes = torch.cuda.mem_get_info()
                allocated = torch.cuda.memory_allocated()
                reserved = torch.cuda.memory_reserved()
                peak_allocated = torch.cuda.max_memory_allocated()
                peak_reserved = torch.cuda.max_memory_reserved()
                gib = 1024**3
                print(
                    "[rocm-memory] "
                    f"step={state.global_step} "
                    f"allocated={allocated / gib:.2f}GiB "
                    f"reserved={reserved / gib:.2f}GiB "
                    f"peak_allocated={peak_allocated / gib:.2f}GiB "
                    f"peak_reserved={peak_reserved / gib:.2f}GiB "
                    f"device_free={free_bytes / gib:.2f}GiB "
                    f"device_total={total_bytes / gib:.2f}GiB",
                    flush=True,
                )
                return control

        callbacks.append(RocmMemoryCallback())

    if args.empty_cache_steps > 0 and torch.cuda.is_available():
        from transformers import TrainerCallback

        class EmptyCacheCallback(TrainerCallback):
            """Release cached ROCm allocations at optimizer-step boundaries."""

            def on_step_end(self, args_, state, control, **kwargs):
                if state.global_step > 0 and state.global_step % args.empty_cache_steps == 0:
                    torch.cuda.empty_cache()
                return control

        callbacks.append(EmptyCacheCallback())

    pad_multiple = args.pad_to_multiple_of if args.pad_to_multiple_of > 0 else None
    base_collator = DataCollatorForSeq2Seq(
        tokenizer,
        model=model,
        pad_to_multiple_of=pad_multiple,
        label_pad_token_id=-100,
        return_tensors="pt",
        padding=True,
    )

    class TracingCollator:
        """Remove diagnostic row IDs before padding the model inputs."""

        def __init__(self, base: Any) -> None:
            self.base = base

        def __call__(self, features: list[dict[str, Any]]) -> dict[str, Any]:
            copied = [dict(feature) for feature in features]
            row_ids = [feature.pop("__row_id__") for feature in copied]
            source_lines = [feature.pop("__source_line__") for feature in copied]
            batch = self.base(copied)
            if args.trace_batch_rows:
                batch["__row_id__"] = row_ids
                batch["__source_line__"] = source_lines
            return batch

    class ReplayTrainer(Trainer):
        """Use a sequential sampler when deterministic replay is requested."""
        def get_train_dataloader(self):
            if args.shuffle_data:
                return super().get_train_dataloader()
            from torch.utils.data import DataLoader, SequentialSampler

            batch_size = getattr(
                self,
                "_train_batch_size",
                self.args.per_device_train_batch_size,
            )
            return DataLoader(
                self.train_dataset,
                batch_size=batch_size,
                sampler=SequentialSampler(self.train_dataset),
                collate_fn=self.data_collator,
                drop_last=self.args.dataloader_drop_last,
                num_workers=self.args.dataloader_num_workers,
                pin_memory=self.args.dataloader_pin_memory,
                persistent_workers=(
                    self.args.dataloader_persistent_workers
                    if self.args.dataloader_num_workers > 0
                    else False
                ),
            )

    class TracingTrainer(ReplayTrainer):
        """Bracket each traced microbatch with synchronizing diagnostics."""

        def training_step(
            self,
            model: Any,
            inputs: dict[str, Any],
            num_items_in_batch: Any = None,
        ):
            row_ids = inputs.pop("__row_id__", None)
            source_lines = inputs.pop("__source_line__", None)
            row_list = trace_values(row_ids)
            source_list = trace_values(source_lines)
            input_shape = (
                tuple(inputs["input_ids"].shape)
                if "input_ids" in inputs and hasattr(inputs["input_ids"], "shape")
                else None
            )
            label_shape = (
                tuple(inputs["labels"].shape)
                if "labels" in inputs and hasattr(inputs["labels"], "shape")
                else None
            )
            attention_shape = (
                tuple(inputs["attention_mask"].shape)
                if "attention_mask" in inputs
                and hasattr(inputs["attention_mask"], "shape")
                else None
            )
            if args.trace_batch_rows and torch.cuda.is_available():
                torch.cuda.synchronize()
            if args.trace_batch_rows:
                print(
                    f"[forward-enter] rows={row_list} source_lines={source_list} "
                    f"input_shape={input_shape} label_shape={label_shape} "
                    f"attention_shape={attention_shape}",
                    flush=True,
                )
            kwargs = {}
            compute_loss_params = inspect.signature(self.compute_loss).parameters
            if "num_items_in_batch" in compute_loss_params:
                kwargs["num_items_in_batch"] = num_items_in_batch
            model.train()
            inputs = self._prepare_inputs(inputs)
            with self.compute_loss_context_manager():
                loss = self.compute_loss(model, inputs, **kwargs)
            if torch.cuda.is_available():
                torch.cuda.synchronize()
            if args.trace_batch_rows:
                print(
                    f"[forward-exit] rows={row_list} source_lines={source_list} "
                    f"loss={float(loss.detach().cpu()):.6f}",
                    flush=True,
                )
            if getattr(self.args, "n_gpu", 1) > 1:
                loss = loss.mean()
            loss_for_backward = loss
            if self.args.gradient_accumulation_steps > 1:
                loss_for_backward = loss_for_backward / self.args.gradient_accumulation_steps
            if args.trace_batch_rows:
                print(
                    f"[backward-enter] rows={row_list} source_lines={source_list}",
                    flush=True,
                )
            self.accelerator.backward(loss_for_backward)
            if args.trace_batch_rows and torch.cuda.is_available():
                torch.cuda.synchronize()
            if args.trace_batch_rows:
                print(
                    f"[backward-exit] rows={row_list} source_lines={source_list}",
                    flush=True,
                )
            return loss.detach()

    trainer_class = TracingTrainer if args.trace_batch_rows else ReplayTrainer
    trainer = trainer_class(
        model=model,
        train_dataset=dataset,
        args=training_args,
        data_collator=TracingCollator(base_collator),
        callbacks=callbacks,
    )

    model.print_trainable_parameters()
    print("[status] starting QLoRA adapter training")
    trainer.train()
    print("[status] saving adapter")
    trainer.save_model(str(out_dir))
    tokenizer.save_pretrained(str(out_dir))
    write_manifest(
        out_dir,
        args,
        len(dataset),
        torch.__version__,
        transformers.__version__,
        peft.__version__,
    )
    print("[status] complete")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
