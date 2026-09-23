#!/usr/bin/env python3
"""Model-free tests for the controlled QLoRA lifecycle helpers."""

from __future__ import annotations

import json
import math
import tempfile
import unittest
from argparse import Namespace
from pathlib import Path
from types import SimpleNamespace

from qlora_lifecycle import (
    authorization_hash,
    collect_training_metrics,
    ensure_controlled_paths,
    finite_metrics,
    lifecycle_mode,
    safetensors_inventory,
    sha256_file,
    validate_authorization,
    verify_canonical_source,
    write_json_exclusive,
)


HASH_FIXTURE = "48d793c13ce4a99dc684ff255e94b3acc5c61a9cb128add80d8528876550a41d"


def authorization() -> dict[str, object]:
    """Return the fixture shared with lmml-substrate's Rust tests."""
    return {
        "schema_version": 2,
        "authorization_id": "authorize-1",
        "base_lineage_id": "qwen38-27b",
        "base_manifest_hash": "a" * 64,
        "config_hash": "b" * 64,
        "tokenizer_hash": "c" * 64,
        "dataset_hash": "d" * 64,
        "seed": 42,
        "training_config": {},
        "approved_allowlist": ["model.layers.*"],
        "trainable_parameters": [
            "model.layers.0.self_attn.q_proj.lora_A",
        ],
        "created_at": "2026-08-25T00:00:00Z",
    }


def file_record(path: Path) -> dict[str, object]:
    """Create one canonical file record for a test checkpoint."""
    return {
        "path": path.name,
        "size_bytes": path.stat().st_size,
        "sha256": sha256_file(path),
    }


def lifecycle_args(**overrides: object) -> Namespace:
    """Construct only the arguments used by lifecycle-mode validation."""
    values = {
        "base_manifest": "",
        "authorization_manifest": "",
        "authorization_draft": "",
        "authorization_id": "",
        "training_run_id": "",
        "adapter_artifact_id": "",
        "training_report": "",
        "adapter_manifest_path": "",
        "prepare_only": False,
        "save_strategy": "no",
        "seed": 42,
    }
    values.update(overrides)
    return Namespace(**values)


class QloraLifecycleTests(unittest.TestCase):
    """Exercise lifecycle gates without importing Torch or touching a GPU."""

    def test_authorization_hash_matches_rust_fixture(self) -> None:
        record = authorization()
        self.assertEqual(authorization_hash(record), HASH_FIXTURE)
        record["training_config"] = {
            "z": {"number": 1.0, "enabled": True},
            "a": ["x", 3],
        }
        self.assertEqual(
            authorization_hash(record),
            "1c78c4525696456a90c9332af4cb3ff545ffe95687949c1835bcbb84a8130ea1",
        )

    def test_canonical_source_rejects_changed_and_stale_shards(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            config = root / "config.json"
            index = root / "model.safetensors.index.json"
            shard = root / "model-00001-of-00001.safetensors"
            tokenizer = root / "tokenizer.json"
            config.write_text("{}\n", encoding="utf-8")
            index.write_text("{}\n", encoding="utf-8")
            shard.write_bytes(b"weights")
            tokenizer.write_text("{}\n", encoding="utf-8")
            manifest = {
                "schema_version": 2,
                "config_hash": sha256_file(config),
                "index_hash": sha256_file(index),
                "shards": [{**file_record(shard), "tensor_names": ["weight"]}],
                "auxiliary_files": [file_record(tokenizer)],
            }

            verify_canonical_source(root, manifest)
            shard.write_bytes(b"changed")
            with self.assertRaisesRegex(ValueError, "size changed|hash changed"):
                verify_canonical_source(root, manifest)
            shard.write_bytes(b"weights")
            (root / "stale.safetensors").write_bytes(b"stale")
            with self.assertRaisesRegex(ValueError, "stale or missing"):
                verify_canonical_source(root, manifest)

    def test_unexpected_trainable_parameter_is_rejected(self) -> None:
        base = {
            "model": {
                "lineage_id": "qwen38-27b",
                "canonical_manifest_hash": "a" * 64,
            },
            "config_hash": "b" * 64,
            "tokenizer_hash": "c" * 64,
        }
        record = authorization()
        record["trainable_parameters"] = ["model.layers.0.self_attn.q_proj.weight"]
        record["approved_allowlist"] = ["model.layers.*"]
        with self.assertRaisesRegex(ValueError, "unexpected trainable base"):
            validate_authorization(
                base,
                record,
                "d" * 64,
                42,
                {},
                record["trainable_parameters"],
            )

    def test_finite_metrics_require_loss_and_gradient_evidence(self) -> None:
        finite_metrics(1.0, [0.5])
        for loss, gradients in (
            (math.nan, [0.5]),
            (1.0, []),
            (1.0, [math.inf]),
        ):
            with self.assertRaises(ValueError):
                finite_metrics(loss, gradients)

    def test_training_metric_collection_requires_logged_gradient_norms(self) -> None:
        result = SimpleNamespace(metrics={"train_loss": 0.75})
        self.assertEqual(
            collect_training_metrics(result, [{"grad_norm": 0.25}]),
            (0.75, [0.25]),
        )
        with self.assertRaisesRegex(ValueError, "gradient norm"):
            collect_training_metrics(result, [])

    def test_controlled_paths_reject_reuse_and_checkpoint_overlap(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            model = root / "model"
            model.mkdir()
            args = Namespace(
                authorization_draft="",
                training_report=str(root / "evidence" / "run.json"),
            )
            output = root / "adapter"
            ensure_controlled_paths("train", args, model, output)
            (output / "partial").write_text("partial", encoding="utf-8")
            with self.assertRaisesRegex(FileExistsError, "not empty"):
                ensure_controlled_paths("train", args, model, output)
            with self.assertRaisesRegex(ValueError, "overlaps"):
                ensure_controlled_paths("train", args, model, model / "output")

    def test_safetensors_inventory_reads_tensor_header(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "adapter_model.safetensors"
            header = json.dumps(
                {"adapter.weight": {"dtype": "F32", "shape": [1], "data_offsets": [0, 4]}},
                separators=(",", ":"),
            ).encode("utf-8")
            path.write_bytes(len(header).to_bytes(8, "little") + header + b"\0\0\0\0")
            self.assertEqual(
                safetensors_inventory(path),
                [
                    {
                        "name": "adapter.weight",
                        "shard": "adapter_model.safetensors",
                        "dtype": "F32",
                        "shape": [1],
                        "parameter_count": 1,
                    }
                ],
            )

    def test_exclusive_json_refuses_overwrite(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "evidence.json"
            write_json_exclusive(path, {"value": 1})
            with self.assertRaises(FileExistsError):
                write_json_exclusive(path, {"value": 2})
            self.assertEqual(sorted(item.name for item in path.parent.iterdir()), [path.name])

    def test_lifecycle_mode_requires_complete_controlled_inputs(self) -> None:
        self.assertEqual(lifecycle_mode(lifecycle_args()), "legacy")
        self.assertEqual(
            lifecycle_mode(
                lifecycle_args(
                    base_manifest="base.json",
                    authorization_draft="draft.json",
                    authorization_id="authorize-1",
                )
            ),
            "draft",
        )
        self.assertEqual(
            lifecycle_mode(
                lifecycle_args(
                    base_manifest="base.json",
                    authorization_manifest="authorization.json",
                    training_run_id="run-1",
                    adapter_artifact_id="adapter-1",
                    training_report="run.json",
                )
            ),
            "train",
        )
        with self.assertRaisesRegex(ValueError, "missing lifecycle fields"):
            lifecycle_mode(lifecycle_args(base_manifest="base.json"))
        with self.assertRaisesRegex(ValueError, "save-strategy no"):
            lifecycle_mode(
                lifecycle_args(
                    base_manifest="base.json",
                    authorization_draft="draft.json",
                    authorization_id="authorize-1",
                    save_strategy="steps",
                )
            )


if __name__ == "__main__":
    unittest.main()
