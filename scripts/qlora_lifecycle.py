#!/usr/bin/env python3
"""Training-side identity and evidence helpers for LMML successor runs."""

from __future__ import annotations

import hashlib
import json
import math
import os
import tempfile
from collections import OrderedDict
from datetime import datetime, timezone
from importlib.metadata import PackageNotFoundError, version
from pathlib import Path
from typing import Any


SCHEMA_VERSION = 2
AUTHORIZATION_FIELDS = (
    "schema_version",
    "authorization_id",
    "base_lineage_id",
    "base_manifest_hash",
    "config_hash",
    "tokenizer_hash",
    "dataset_hash",
    "seed",
    "training_config",
    "approved_allowlist",
    "trainable_parameters",
    "created_at",
)
IDENTIFIER_CHARS = frozenset(
    "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_."
)


def sha256_file(path: Path) -> str:
    """Return the SHA-256 digest for one file."""
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(4 * 1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_json_object(path: Path) -> dict[str, Any]:
    """Load a JSON object and reject other top-level values."""
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"expected a JSON object in {path}")
    return value


def _sorted_value(value: Any) -> Any:
    if isinstance(value, dict):
        return OrderedDict((key, _sorted_value(value[key])) for key in sorted(value))
    if isinstance(value, list):
        return [_sorted_value(item) for item in value]
    return value


def authorization_hash(authorization: dict[str, Any]) -> str:
    """Match LMML's stable JSON hash for a validated authorization record."""
    missing = [field for field in AUTHORIZATION_FIELDS if field not in authorization]
    if missing:
        raise ValueError(f"authorization is missing fields: {', '.join(missing)}")
    ordered = OrderedDict(
        (field, _sorted_value(authorization[field])) for field in AUTHORIZATION_FIELDS
    )
    payload = json.dumps(
        ordered,
        ensure_ascii=False,
        separators=(",", ":"),
        allow_nan=False,
    ).encode("utf-8")
    return hashlib.sha256(payload).hexdigest()


def _require_identifier(value: str, label: str) -> None:
    if not value or value in {".", ".."} or any(
        character not in IDENTIFIER_CHARS for character in value
    ):
        raise ValueError(f"{label} is not a valid LMML identifier: {value!r}")


def lifecycle_mode(args: Any) -> str:
    """Validate lifecycle arguments and return legacy, draft, or train mode."""
    fields = {
        "base_manifest": args.base_manifest,
        "authorization_manifest": args.authorization_manifest,
        "authorization_draft": args.authorization_draft,
        "authorization_id": args.authorization_id,
        "training_run_id": args.training_run_id,
        "adapter_artifact_id": args.adapter_artifact_id,
        "training_report": args.training_report,
        "adapter_manifest_path": args.adapter_manifest_path,
    }
    populated = {name for name, value in fields.items() if value}
    if not populated:
        return "legacy"
    if args.prepare_only:
        raise ValueError("prepare-only cannot be combined with lifecycle mode")
    if args.save_strategy != "no":
        raise ValueError("lifecycle training requires --save-strategy no")
    if args.seed < 0 or args.seed > (1 << 64) - 1:
        raise ValueError("lifecycle seed must fit an unsigned 64-bit integer")

    if args.authorization_draft:
        required = {"base_manifest", "authorization_draft", "authorization_id"}
        forbidden = {
            "authorization_manifest",
            "training_run_id",
            "adapter_artifact_id",
            "training_report",
            "adapter_manifest_path",
        }
        if missing := required - populated:
            raise ValueError(
                "authorization draft is missing lifecycle fields: "
                + ", ".join(sorted(missing))
            )
        if present := forbidden & populated:
            raise ValueError(
                "authorization draft received training-only fields: "
                + ", ".join(sorted(present))
            )
        _require_identifier(args.authorization_id, "authorization ID")
        return "draft"

    required = {
        "base_manifest",
        "authorization_manifest",
        "training_run_id",
        "adapter_artifact_id",
        "training_report",
    }
    forbidden = {"authorization_draft", "authorization_id"}
    if missing := required - populated:
        raise ValueError(
            "controlled training is missing lifecycle fields: "
            + ", ".join(sorted(missing))
        )
    if present := forbidden & populated:
        raise ValueError(
            "controlled training received draft-only fields: "
            + ", ".join(sorted(present))
        )
    _require_identifier(args.training_run_id, "training run ID")
    _require_identifier(args.adapter_artifact_id, "adapter artifact ID")
    return "train"


def training_config(
    args: Any,
    *,
    dataset_rows: int,
    effective_blas_backend: str,
    effective_compute_dtype: str,
    package_versions: dict[str, str],
) -> dict[str, Any]:
    """Return the complete behavior-affecting configuration for authorization."""
    return {
        "attention": {
            "implementation": args.attn_implementation,
            "force_math_sdp": args.force_math_sdp,
        },
        "data": {
            "effective_rows": dataset_rows,
            "max_samples": args.max_samples,
            "only_source_lines": sorted(_parse_indices(args.only_source_lines)),
            "pad_to_multiple_of": args.pad_to_multiple_of,
            "sequence_length": args.seq_len,
            "shuffle": args.shuffle_data,
        },
        "dataloader": {
            "num_workers": args.dataloader_num_workers,
            "persistent_workers": args.dataloader_persistent_workers,
            "pin_memory": args.dataloader_pin_memory,
        },
        "diagnostics": {
            "backward_trace_markers": _parse_markers(args.backward_trace_markers),
            "log_memory_steps": args.log_memory_steps,
            "trace_backward_modules": args.trace_backward_modules,
            "trace_backward_prefix": args.trace_backward_prefix,
            "trace_batch_rows": args.trace_batch_rows,
        },
        "lora": {
            "alpha": args.lora_alpha,
            "autocast_adapter_dtype": args.lora_autocast_adapter_dtype,
            "bias": "none",
            "dropout": args.lora_dropout,
            "exclude_modules": args.lora_exclude_modules,
            "rank": args.lora_r,
            "target_modules": _parse_markers(args.target_modules),
            "task_type": "CAUSAL_LM",
        },
        "model_load": {
            "device_map": "auto",
            "low_cpu_mem_usage": True,
            "trust_remote_code": True,
            "use_cache": False,
        },
        "optimization": {
            "epochs": args.epochs,
            "gradient_accumulation_steps": args.grad_accum,
            "learning_rate": args.learning_rate,
            "lr_scheduler_type": "cosine",
            "max_grad_norm": 1.0,
            "max_steps": args.max_steps,
            "optimizer": args.optim,
            "per_device_train_batch_size": 1,
            "warmup_steps": 10,
        },
        "packages": dict(sorted(package_versions.items())),
        "quantization": {
            "compute_dtype": effective_compute_dtype,
            "double_quant": True,
            "load_in_4bit": True,
            "requested_compute_dtype": args.compute_dtype,
            "type": "nf4",
        },
        "runtime": {
            "empty_cache_steps": args.empty_cache_steps,
            "rocblas_use_hipblaslt": os.getenv("ROCBLAS_USE_HIPBLASLT", ""),
            "rocm_blas_backend": args.rocm_blas_backend,
            "selected_blas_backend": effective_blas_backend,
            "save_strategy": args.save_strategy,
            "torch_blas_prefer_hipblaslt": os.getenv(
                "TORCH_BLAS_PREFER_HIPBLASLT", ""
            ),
            "container_image": os.getenv("LMML_QLORA_IMAGE", ""),
            "container_image_id": os.getenv("LMML_QLORA_IMAGE_ID", ""),
            "container_runtime": os.getenv("LMML_CONTAINER_RUNTIME_VERSION", ""),
        },
        "trainer": {
            "gradient_checkpointing": True,
            "gradient_checkpointing_reentrant": False,
            "label_pad_token_id": -100,
            "logging_steps": 1,
            "logging_strategy": "steps",
            "remove_unused_columns": False,
            "report_to": "none",
            "tokenizer_padding_side": "right",
        },
    }


def package_versions(
    torch_module: Any,
    transformers_module: Any,
    peft_module: Any,
    trainer_path: Path,
) -> dict[str, str]:
    """Capture the exact training packages bound into an authorization."""
    try:
        bitsandbytes_version = version("bitsandbytes")
    except PackageNotFoundError:
        bitsandbytes_version = "unknown"
    versions = {
        "bitsandbytes": bitsandbytes_version,
        "lifecycle_helper_sha256": sha256_file(Path(__file__)),
        "peft": str(peft_module.__version__),
        "torch": str(torch_module.__version__),
        "torch_hip": str(getattr(torch_module.version, "hip", None)),
        "trainer_sha256": sha256_file(trainer_path),
        "transformers": str(transformers_module.__version__),
    }
    if torch_module.cuda.is_available():
        properties = torch_module.cuda.get_device_properties(0)
        versions["device"] = str(torch_module.cuda.get_device_name(0))
        versions["gcn_arch"] = str(getattr(properties, "gcnArchName", "unknown"))
    else:
        versions["device"] = "unavailable"
        versions["gcn_arch"] = "unavailable"
    return versions


def ensure_controlled_paths(mode: str, args: Any, model_id: Path, out_dir: Path) -> None:
    """Reject destructive or reusable paths before loading the model."""
    if mode == "legacy":
        out_dir.mkdir(parents=True, exist_ok=True)
        return

    model_root = model_id.resolve()
    evidence_path = Path(
        args.authorization_draft if mode == "draft" else args.training_report
    )
    for label, path in (("output directory", out_dir), ("evidence", evidence_path)):
        resolved = path.resolve()
        if (
            resolved == model_root
            or model_root in resolved.parents
            or resolved in model_root.parents
        ):
            raise ValueError(f"controlled {label} overlaps the canonical checkpoint")
    if evidence_path.exists():
        raise FileExistsError(f"controlled evidence already exists: {evidence_path}")
    if mode == "train":
        if out_dir.exists() and any(out_dir.iterdir()):
            raise FileExistsError(
                f"controlled adapter output directory is not empty: {out_dir}"
            )
        out_dir.mkdir(parents=True, exist_ok=True)


def collect_training_metrics(
    train_result: Any,
    log_history: list[dict[str, Any]],
) -> tuple[float, list[float]]:
    """Extract finite final-loss and gradient-norm evidence from Trainer."""
    raw_loss = train_result.metrics.get("train_loss")
    if raw_loss is None:
        losses = [entry["loss"] for entry in log_history if "loss" in entry]
        if not losses:
            raise ValueError("trainer produced no final loss evidence")
        raw_loss = losses[-1]
    gradient_norms = [
        float(entry["grad_norm"])
        for entry in log_history
        if entry.get("grad_norm") is not None
    ]
    final_loss = float(raw_loss)
    finite_metrics(final_loss, gradient_norms)
    return final_loss, gradient_norms


def validate_live_adapter_tensors(model: Any, torch_module: Any) -> None:
    """Reject non-finite trainable adapter tensors before serialization."""
    checked = 0
    for name, parameter in model.named_parameters():
        if not parameter.requires_grad:
            continue
        checked += 1
        if not bool(torch_module.isfinite(parameter.detach()).all().item()):
            raise ValueError(f"adapter tensor is non-finite before save: {name}")
    if checked == 0:
        raise ValueError("model has no trainable adapter tensors before save")


def validate_serialized_adapter(path: Path) -> list[dict[str, Any]]:
    """Inspect and finite-check the saved adapter payload."""
    from safetensors.torch import load_file

    tensors = load_file(str(path), device="cpu")
    if not tensors:
        raise ValueError("saved adapter contains no tensors")
    import torch

    for name, tensor in tensors.items():
        if tensor.is_floating_point() or tensor.is_complex():
            if not bool(torch.isfinite(tensor).all().item()):
                raise ValueError(f"saved adapter tensor is non-finite: {name}")
    return safetensors_inventory(path)


def _parse_markers(value: str) -> list[str]:
    return [marker.strip() for marker in value.split(",") if marker.strip()]


def _parse_indices(value: str) -> set[int]:
    return {int(item.strip()) for item in value.split(",") if item.strip()}


def build_authorization(
    base: dict[str, Any],
    *,
    authorization_id: str,
    dataset_hash: str,
    seed: int,
    config: dict[str, Any],
    trainable_parameters: list[str],
) -> dict[str, Any]:
    """Build the proposed immutable authorization for a configured LoRA model."""
    model = base.get("model")
    if not isinstance(model, dict):
        raise ValueError("canonical manifest has no model identity")
    allowlist = [parameter for parameter in trainable_parameters]
    authorization = {
        "schema_version": SCHEMA_VERSION,
        "authorization_id": authorization_id,
        "base_lineage_id": model.get("lineage_id"),
        "base_manifest_hash": model.get("canonical_manifest_hash"),
        "config_hash": base.get("config_hash"),
        "tokenizer_hash": base.get("tokenizer_hash"),
        "dataset_hash": dataset_hash,
        "seed": seed,
        "training_config": config,
        "approved_allowlist": allowlist,
        "trainable_parameters": trainable_parameters,
        "created_at": rfc3339_now(),
    }
    validate_authorization(
        base,
        authorization,
        dataset_hash,
        seed,
        config,
        trainable_parameters,
    )
    return authorization


def build_training_report(
    base: dict[str, Any],
    authorization: dict[str, Any],
    *,
    training_run_id: str,
    adapter_artifact_id: str,
    adapter_path: Path,
    adapter_hash: str,
    adapter_tensors: list[dict[str, Any]],
    final_loss: float,
    gradient_norms: list[float],
) -> dict[str, Any]:
    """Build an LMML schema-v2 completed training-run report."""
    finite_metrics(final_loss, gradient_norms)
    model = base["model"]
    return {
        "schema_version": SCHEMA_VERSION,
        "training_run_id": training_run_id,
        "authorization_id": authorization["authorization_id"],
        "authorization_hash": authorization_hash(authorization),
        "base_lineage_id": authorization["base_lineage_id"],
        "base_manifest_hash": authorization["base_manifest_hash"],
        "config_hash": authorization["config_hash"],
        "tokenizer_hash": authorization["tokenizer_hash"],
        "dataset_hash": authorization["dataset_hash"],
        "seed": authorization["seed"],
        "training_config": authorization["training_config"],
        "approved_allowlist": authorization["approved_allowlist"],
        "trainable_parameters": authorization["trainable_parameters"],
        "final_loss": final_loss,
        "gradient_norms": gradient_norms,
        "adapter": {
            "artifact_id": adapter_artifact_id,
            "model_lineage_id": model["lineage_id"],
            "representation": "safetensors",
            "quantization": None,
            "artifact_hash": adapter_hash,
        },
        "adapter_path": str(adapter_path),
        "adapter_tensors": adapter_tensors,
        "created_at": rfc3339_now(),
    }


def _verify_file(root: Path, record: dict[str, Any]) -> None:
    relative = record.get("path")
    if not isinstance(relative, str) or not relative:
        raise ValueError("canonical manifest contains an invalid file path")
    path = root / relative
    if not path.is_file():
        raise ValueError(f"canonical model file is missing: {path}")
    expected_size = record.get("size_bytes")
    if path.stat().st_size != expected_size:
        raise ValueError(f"canonical model file size changed: {path}")
    if sha256_file(path) != record.get("sha256"):
        raise ValueError(f"canonical model file hash changed: {path}")


def verify_canonical_source(root: Path, manifest: dict[str, Any]) -> None:
    """Re-hash the selected checkpoint against its canonical LMML manifest."""
    if manifest.get("schema_version") != SCHEMA_VERSION:
        raise ValueError("unsupported canonical substrate schema")
    config = root / "config.json"
    if not config.is_file() or sha256_file(config) != manifest.get("config_hash"):
        raise ValueError("selected training base does not match canonical config.json")
    index_hash = manifest.get("index_hash")
    index = root / "model.safetensors.index.json"
    if index_hash is not None and (
        not index.is_file() or sha256_file(index) != index_hash
    ):
        raise ValueError("selected training base does not match canonical index")

    shards = manifest.get("shards")
    auxiliary = manifest.get("auxiliary_files")
    if not isinstance(shards, list) or not shards:
        raise ValueError("canonical manifest has no Safetensors shards")
    if not isinstance(auxiliary, list):
        raise ValueError("canonical manifest has no auxiliary-file inventory")
    for record in [*shards, *auxiliary]:
        if not isinstance(record, dict):
            raise ValueError("canonical manifest contains an invalid file record")
        _verify_file(root, record)

    expected_shards = {record["path"] for record in shards}
    actual_shards = {path.name for path in root.glob("*.safetensors")}
    if actual_shards != expected_shards:
        raise ValueError("selected training base contains stale or missing shards")


def _matches_allowlist(parameter: str, allowed: str) -> bool:
    if allowed.endswith("*"):
        return parameter.startswith(allowed[:-1])
    return parameter == allowed


def validate_authorization(
    base: dict[str, Any],
    authorization: dict[str, Any],
    dataset_hash: str,
    seed: int,
    training_config: dict[str, Any],
    trainable_parameters: list[str],
) -> None:
    """Reject any runtime training state that differs from its authorization."""
    if authorization.get("schema_version") != SCHEMA_VERSION:
        raise ValueError("unsupported training authorization schema")
    authorization_id = authorization.get("authorization_id")
    if not isinstance(authorization_id, str):
        raise ValueError("training authorization has no ID")
    _require_identifier(authorization_id, "authorization ID")
    created_at = authorization.get("created_at")
    if not isinstance(created_at, str):
        raise ValueError("training authorization has no creation timestamp")
    try:
        timestamp = datetime.fromisoformat(created_at.replace("Z", "+00:00"))
    except ValueError as error:
        raise ValueError("training authorization timestamp is not RFC3339") from error
    if timestamp.utcoffset() is None:
        raise ValueError("training authorization timestamp requires a UTC offset")
    for field in (
        "base_manifest_hash",
        "config_hash",
        "tokenizer_hash",
        "dataset_hash",
    ):
        value = authorization.get(field)
        if not isinstance(value, str) or len(value) != 64 or any(
            character not in "0123456789abcdef" for character in value
        ):
            raise ValueError(f"training authorization has an invalid {field}")
    model = base.get("model")
    if not isinstance(model, dict):
        raise ValueError("canonical manifest has no model identity")
    expected = {
        "schema_version": SCHEMA_VERSION,
        "base_lineage_id": model.get("lineage_id"),
        "base_manifest_hash": model.get("canonical_manifest_hash"),
        "config_hash": base.get("config_hash"),
        "tokenizer_hash": base.get("tokenizer_hash"),
        "dataset_hash": dataset_hash,
        "seed": seed,
        "training_config": training_config,
        "trainable_parameters": trainable_parameters,
    }
    for field, value in expected.items():
        if authorization.get(field) != value:
            raise ValueError(f"training authorization mismatch: {field}")

    allowlist = authorization.get("approved_allowlist")
    if not isinstance(allowlist, list) or not allowlist:
        raise ValueError("training authorization requires an allowlist")
    if len(set(allowlist)) != len(allowlist):
        raise ValueError("training authorization allowlist contains duplicates")
    if not trainable_parameters or len(set(trainable_parameters)) != len(
        trainable_parameters
    ):
        raise ValueError("trainable parameter inventory is empty or duplicated")
    lora_markers = (
        ".lora_A",
        ".lora_B",
        ".lora_embedding_A",
        ".lora_embedding_B",
    )
    for parameter in trainable_parameters:
        if not any(marker in parameter for marker in lora_markers):
            raise ValueError(f"unexpected trainable base parameter: {parameter}")
        if not any(_matches_allowlist(parameter, allowed) for allowed in allowlist):
            raise ValueError(f"trainable parameter is outside the allowlist: {parameter}")


def safetensors_inventory(path: Path) -> list[dict[str, Any]]:
    """Read the adapter Safetensors header into LMML tensor descriptors."""
    with path.open("rb") as handle:
        raw_length = handle.read(8)
        if len(raw_length) != 8:
            raise ValueError(f"truncated Safetensors header: {path}")
        header_length = int.from_bytes(raw_length, "little")
        header_bytes = handle.read(header_length)
        if len(header_bytes) != header_length:
            raise ValueError(f"truncated Safetensors metadata: {path}")
    header = json.loads(header_bytes)
    if not isinstance(header, dict):
        raise ValueError(f"invalid Safetensors metadata: {path}")
    tensors: list[dict[str, Any]] = []
    for name in sorted(key for key in header if key != "__metadata__"):
        record = header[name]
        if not isinstance(record, dict):
            raise ValueError(f"invalid tensor metadata for {name}")
        shape = record.get("shape")
        dtype = record.get("dtype")
        offsets = record.get("data_offsets")
        if (
            not isinstance(shape, list)
            or not all(isinstance(size, int) and size >= 0 for size in shape)
            or not isinstance(dtype, str)
            or not isinstance(offsets, list)
            or len(offsets) != 2
        ):
            raise ValueError(f"invalid tensor metadata for {name}")
        tensors.append(
            {
                "name": name,
                "shard": path.name,
                "dtype": dtype,
                "shape": shape,
                "parameter_count": math.prod(shape),
            }
        )
    if not tensors:
        raise ValueError("adapter contains no tensors")
    return tensors


def finite_metrics(final_loss: float, gradient_norms: list[float]) -> None:
    """Require finite loss and gradient evidence before saving an adapter."""
    if not math.isfinite(final_loss):
        raise ValueError("training loss is NaN or infinity")
    if not gradient_norms or not all(math.isfinite(value) for value in gradient_norms):
        raise ValueError("gradient norm evidence is missing or non-finite")


def rfc3339_now() -> str:
    """Return a UTC RFC3339 timestamp accepted by LMML."""
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def write_json_exclusive(path: Path, value: dict[str, Any]) -> None:
    """Atomically persist JSON without replacing an existing evidence record."""
    path.parent.mkdir(parents=True, exist_ok=True)
    payload = (
        json.dumps(value, indent=2, sort_keys=True, allow_nan=False) + "\n"
    ).encode("utf-8")
    descriptor, temporary = tempfile.mkstemp(
        dir=path.parent,
        prefix=f".{path.name}.",
    )
    temporary_path = Path(temporary)
    try:
        os.fchmod(descriptor, 0o600)
        with os.fdopen(descriptor, "wb") as handle:
            handle.write(payload)
            handle.flush()
            os.fsync(handle.fileno())
        os.link(temporary_path, path)
        directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        temporary_path.unlink(missing_ok=True)
