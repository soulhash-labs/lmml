# Qwen3.8 Model Lifecycle

LMML treats `/home/angelo/repos/qwen38-safetensors/` as the canonical Qwen3.8
substrate. The directory must contain the complete checkpoint before import;
Hugging Face partial-download cache files are not model shards.

## Import and verify

```sh
LMML_DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"

lmml model import /home/angelo/repos/qwen38-safetensors \
  --lineage-id qwen38-27b
lmml model inspect "$LMML_DATA_HOME/lmml/models/manifests/qwen38-27b.json"
lmml model verify /home/angelo/repos/qwen38-safetensors \
  --manifest "$LMML_DATA_HOME/lmml/models/manifests/qwen38-27b.json"
lmml model lineage "$LMML_DATA_HOME/lmml/models/manifests/qwen38-27b.json"
```

The importer reads `config.json`, tokenizer/generation/processor assets, the
Safetensors index, every referenced shard, and each shard header. It records
SHA-256 digests, tensor names, dtypes, shapes, parameter count, data layout,
and the architecture fields present in the checkpoint. Index and header tensor
sets must agree exactly, and tensor offsets must cover the complete shard
payload without gaps, overlap, truncation, or trailing data.

Schema version 2 records each behavioral auxiliary asset separately and uses
validated SHA-256 values. During a full import, progress is emitted to stderr
after every hashed weight shard; JSON output on stdout remains machine-readable.
Schema-v1 manifests must be re-imported to a new managed record rather than
edited in place.

Managed lineage registration is immutable. Re-importing identical content is
idempotent, while different content under an existing lineage ID is rejected.
Successor imports require an explicit parent:

```sh
lmml model import /models/qwen38-successor-1 \
  --lineage-id qwen38-successor-1 \
  --parent qwen38-27b
```

The persisted manifest is the source record for later derivatives. A GGUF
file has a separate `ArtifactIdentity` and must point back to the Qwen3.8
`ModelIdentity`; quantization never replaces the Safetensors identity.

## GGUF derivation

Derivation verifies the canonical source before invoking llama.cpp. It runs the
converter and quantizer asynchronously with streamed output, writes to unique
temporary files, inspects GGUF metadata, asks `llama-server` to open and serve
the artifact, refuses to overwrite an output, hashes the result, and appends a
source-artifact anchor plus the derived artifact under the managed data
directory.

```sh
lmml model derive \
  --manifest ~/.local/share/lmml/models/manifests/qwen38-27b.json \
  --source /home/angelo/repos/qwen38-safetensors \
  --output ~/.local/share/lmml/models/artifacts/gguf/qwen38-27b-q8_0.gguf \
  --artifact-id qwen38-27b-q8_0 \
  --quant q8-0
```

When the inference GPU is reserved, conversion can stop at an immutable pending
candidate. This performs source verification, conversion, quantization, GGUF
metadata inspection, SHA-256 hashing, and no-clobber publication without loading
the model or adding it to the runnable artifact catalog:

```sh
lmml model derive \
  --manifest ~/.local/share/lmml/models/manifests/qwen38-27b.json \
  --source /home/angelo/repos/qwen38-safetensors \
  --output ~/.local/share/lmml/models/candidates/qwen38-27b-bf16.gguf \
  --artifact-id qwen38-27b-bf16 \
  --quant bf16 \
  --defer-admission
```

After the GPU is available, admit that exact candidate. LMML re-hashes the
payload, verifies its canonical source, runs the tensor/backend checks, and only
then creates the runnable artifact record:

```sh
lmml model admit-artifact \
  --candidate ~/.local/share/lmml/models/candidates/qwen38-27b-bf16.json \
  --manifest ~/.local/share/lmml/models/manifests/qwen38-27b.json \
  --source /home/angelo/repos/qwen38-safetensors
```

F16/BF16 conversion requires the llama.cpp converter and its Python
dependencies. Q8_0, Q6_K, and Q4_K_M additionally require `llama-quantize`.
LMML reports missing tools or unsupported conversion failures without
registering an artifact.

When the host does not provide the validated ROCm Python/Torch environment,
use the explicit full-model container executor. It does not install Torch into
the host environment; the image must contain Python, Torch, and the llama.cpp
converter dependencies:

```sh
lmml model derive \
  --manifest ~/.local/share/lmml/models/manifests/qwen38-27b.json \
  --source /home/angelo/repos/qwen38-safetensors \
  --output ~/.local/share/lmml/models/artifacts/gguf/qwen38-27b-f16.gguf \
  --artifact-id qwen38-27b-f16 \
  --quant f16 \
  --rocm-container /usr/bin/docker
```

The default image is AMD's validated
`rocm/pytorch:rocm7.2.4_ubuntu24.04_py3.12_pytorch_release_2.9.1` image. Override
it with `--rocm-image` only when another image has been validated. Before LMML
hashes the substrate or starts conversion, it verifies that the image imports
Torch, NumPy, Safetensors, Transformers, and llama.cpp's GGUF Python module. Provenance records
the container runtime version, image ID/digest, Python/Torch/ROCm package
versions, and the host-mounted converter Git revision.

Conversion does not receive `/dev/kfd` or `/dev/dri`; GGUF conversion is a CPU
operation and must not contend with an active GPU workload. The container mounts
the canonical source read-only and can write only inside a private temporary
derivation directory.
An immediately admitted GGUF is published only after metadata inspection,
`llama-server --check-tensors` admission, and the 600-second large-model startup
window. A deferred candidate is published under the candidate boundary and
cannot enter runtime selection before `model admit-artifact` completes.

List the append-only artifact catalog with:

```sh
lmml model artifacts --lineage-id qwen38-27b --json
```

Artifact records include the exact absolute payload path, SHA-256, conversion
provenance, parent artifact, and backend admission evidence. A GGUF without
admission evidence cannot enter the catalog or runtime selection.

## Pristine baseline

LMML ships a deterministic prompt set at
`docs/fixtures/qwen38-baseline-prompts.json`:

```json
[
  {"case_id":"identity","prompt":"State your model family in one sentence."},
  {"case_id":"arithmetic","prompt":"Return only the result of 37 * 19."}
]
```

Then bind the baseline to one exact admitted artifact:

```sh
lmml model baseline \
  --artifact-id qwen38-27b-q8_0 \
  --baseline-id pristine-v1 \
  --prompts docs/fixtures/qwen38-baseline-prompts.json \
  --predict 32 \
  --gpu-layers -1
```

LMML records the artifact and backend hashes, fixed sampling parameters, prompts,
captured outputs, and per-output SHA-256 values. Use `--gpu-layers 0` for a CPU
baseline. Baseline execution loads the model; do not run it while the selected
device is reserved for another workload.

## Runtime boundary

llama.cpp remains LMML's deployment backend. Register an already-running local
process against its admitted artifact, then request a capability lease:

```sh
lmml runtime register \
  --runtime-id qwen38-q8-local \
  --artifact-id qwen38-27b-q8_0 \
  --pid 12345 \
  --endpoint http://127.0.0.1:1200 \
  --backend-version e79e4bf \
  --context-size 16384

lmml runtime request \
  --lineage-id qwen38-27b \
  --purpose agentq-inference \
  --capability text-generation \
  --lease-id agentq-qwen38-001 \
  --json
```

Registration re-hashes the artifact and verifies that `/proc/<pid>/cmdline`
contains its exact path. A lease returns IDs, hashes, capabilities, and endpoint,
not a filesystem path. Standard llama.cpp advertises text generation only and
returns an explicit unsupported error for hidden-state or CROWN11 tap requests.
An instrumented provider must advertise and return real observations.

## Successor checkpoints

Before optimization, the trainer enumerates its trainable parameters and asks
LMML to persist an authorization containing canonical base hashes, dataset hash,
seed, configuration, allowlist, and exact trainable inventory:

```sh
lmml model authorize-successor-training \
  --base-manifest ~/.local/share/lmml/models/manifests/qwen38-27b.json \
  --authorization training-authorization.json
```

The completed `TrainingRunManifest` must reproduce that authorization and its
SHA-256, then add finite loss/gradient evidence and the adapter path/hash. LMML
requires an exact LoRA tensor inventory and scans payloads for NaN and infinity.
Generate the inventory with `lmml model inspect-adapter ADAPTER --json`, then
register the completed run:

```sh
lmml model train-successor \
  --base-manifest ~/.local/share/lmml/models/manifests/qwen38-27b.json \
  --authorization authorization_manifest.json \
  --report training-run.json
```

Merge only from the canonical unquantized parent. The training-side utility uses
the current Transformers `dtype=` API, writes a new candidate directory, measures
the LoRA delta, and compares live-adapter and merged logits:

```sh
python3 scripts/merge_lora.py \
  --base /home/angelo/repos/qwen38-safetensors \
  --adapter /models/adapters/adapter-1 \
  --output /models/successors/qwen38-successor-1 \
  --report merge-evidence.json \
  --dtype bfloat16 \
  --equivalence-inputs equivalence-inputs.json

lmml model merge-successor \
  --base-manifest ~/.local/share/lmml/models/manifests/qwen38-27b.json \
  --base-source /home/angelo/repos/qwen38-safetensors \
  --training-manifest training-run.json \
  --candidate /models/successors/qwen38-successor-1 \
  --successor-lineage-id qwen38-successor-1 \
  --candidate-id qwen38-successor-1-merge-1 \
  --report merge-evidence.json
```

LMML imports the candidate, requires the explicit parent, verifies all files,
and checks architecture plus every tensor name, dtype, and shape against the
parent. It rejects zero deltas or logit differences above the report tolerance.

Admission requires regression results for exactly the cases in the frozen parent
baseline:

```sh
lmml model admit-successor \
  --candidate-manifest candidate_manifest.json \
  --baseline-manifest pristine-v1/manifest.json \
  --report successor-admission.json
```

Only this final command stores `successor_manifest.json` and the managed
successor substrate. `model derive` refuses a successor lineage without that
admission record. Failed training, merge, equivalence, or regression leaves the
parent untouched.

## Current gates

The code implements the substrate importer, immutable deterministic manifest,
verification path, identity separation, guarded GGUF derivation, append-only
artifact registration, quantized temporary publication, CPU-only container
conversion, artifact-bound runtime leases, deterministic baselines, and typed
successor gates.

The schema-v2 managed manifest now exists for the complete canonical checkpoint,
with canonical hash
`fe6a79f82e8c830c801ac5b82b0e18c19c0d6c1e5626534431b4424a8161e1d0`.
`llama-quantize` is built at managed llama.cpp revision `e79e4bf`. The host now
has an isolated converter layer using ROCm Torch 2.13, Safetensors 0.8,
Transformers 5.14, NumPy 2.5, and llama.cpp's local GGUF module. The first real
F16/BF16 GGUF and its Q8_0, Q6_K, and Q4_K_M derivatives remain to be executed.
Backend admission and baseline execution are also paused while the R9700 is in
use. These pending executions do not weaken the catalog: no derivative is marked
usable until metadata, tensor, backend, hash, and lineage gates pass. No partial
download is eligible for conversion, and no Qwen3.5 asset is used as a fallback.

## Record examples

The canonical model identity and a Q8 deployment artifact remain separate:

```json
{
  "model": {
    "lineage_id": "qwen38-27b",
    "parent": null,
    "canonical_manifest_hash": "fe6a79f82e8c830c801ac5b82b0e18c19c0d6c1e5626534431b4424a8161e1d0"
  },
  "artifact": {
    "artifact_id": "qwen38-27b-q8_0",
    "model_lineage_id": "qwen38-27b",
    "representation": "gguf",
    "quantization": "q8_0",
    "artifact_hash": "<sha256-of-gguf>"
  }
}
```

The derivative chain records one canonical source anchor and independent GGUF
artifacts:

```text
qwen38-27b-safetensors
  -> qwen38-27b-f16
  -> qwen38-27b-q8_0
  -> qwen38-27b-q6_k
  -> qwen38-27b-q4_k_m
```

A lease returns the selected runtime and hashes, without returning the managed
artifact path:

```json
{
  "lease_id": "agentq-qwen38-001",
  "model_lineage_id": "qwen38-27b",
  "artifact_id": "qwen38-27b-q8_0",
  "runtime_id": "qwen38-q8-local",
  "endpoint": "http://127.0.0.1:1200",
  "manifest_hash": "fe6a79f82e8c830c801ac5b82b0e18c19c0d6c1e5626534431b4424a8161e1d0",
  "artifact_hash": "<sha256-of-gguf>",
  "capabilities": ["text_generation"]
}
```

Successor lineage remains explicit:

```text
qwen38-27b
  -> authorization-1
  -> training-run-1
  -> adapter-1
  -> candidate-1 (qwen38-successor-1, parent=qwen38-27b)
  -> successor admission
  -> admitted successor GGUF derivatives
```
