# WORK PROJECT — LMML: Qwen3.8-27B Substrate, GGUF, Runtime, Lineage & Successor Pipeline

## Mission

Upgrade LMML so it becomes the authoritative **model lifecycle, artifact derivation, runtime orchestration, and successor-checkpoint plane** for the CROWN11 Quantum AI stack.

The canonical model source is:

```text
/repos/qwen38-safetensors/
```

Treat **Qwen3.8-27B Safetensors as the sole canonical substrate** for this work. Existing/stale Qwen3.5-9B material is noise for this project and MUST NOT be used to infer architecture, tensor names, block count, hidden sizes, attention topology, tokenizer details, or tap locations.

LMML must manage model representations and runtimes without becoming the CROWN/nMath/PMEP/ASTRA logic layer.

Core doctrine:

```text
Safetensors master = canonical model source
LMML              = model lifecycle + artifact DAG + runtime selection/orchestration
GGUF/llama.cpp    = deployment inference plane
instrumented runner = optional research capability for hidden-state/tap observation
CROWN11/AgentQ    = cognition / semantic / relational / governance plane
```

Do not duplicate CROWN policy/gating logic in LMML.

---

# 0. Non-negotiable architecture boundaries

Preserve these invariants:

1. `ModelIdentity != ArtifactIdentity`.
   - A model lineage identifies the conceptual model/checkpoint.
   - A GGUF Q8/Q6/Q4 file is a derived artifact of that model.

2. `Safetensors != GGUF`.
   - Safetensors is the canonical training/master representation.
   - GGUF is a derived deployment representation.
   - Quantized GGUF must NEVER replace canonical Safetensors identity.

3. `TRAIN != MERGE != ADMIT != QUANTIZE != RUN`.

4. Never overwrite the pristine base checkpoint.

5. Every successor has an explicit parent: `M_(n+1).parent = M_n.id`.

6. Artifact lineage is append-only.

7. LMML must not claim arbitrary-layer activation support from llama.cpp unless the backend actually exposes it.
   - If unsupported, return an explicit capability error.
   - Do not fabricate activation values.

8. No learned CROWN/tap weights are part of this LMML increment.

9. Do not use stale Qwen3.5-9B assets as a fallback.

10. Existing LMML llama.cpp behavior should remain compatible:
    - mmap remains the normal/default model loading path unless configured otherwise;
    - do not introduce an unconditional mlock requirement;
    - preserve existing launch-builder behavior unless a verified change is required.

---

# 1. Shared model-substrate contract

LMML must consume a versioned substrate schema from a dedicated shared crate/module named conceptually:

```text
model-substrate
```

Do NOT clone or independently reinvent the same schema if a shared crate already exists.

If the shared crate does not yet exist in the checkout:
- implement LMML behind a narrow local trait/protocol boundary;
- define serialization fixtures/tests for the expected schema;
- stop short of creating a divergent second canonical schema;
- clearly mark the dependency awaiting the shared crate.

Expected core structures:

```rust
pub struct ModelIdentity {
    pub lineage_id: String,
    pub model_name: String,
    pub parent: Option<String>,
    pub canonical_manifest_hash: Hash256,
}

pub struct ArtifactIdentity {
    pub artifact_id: String,
    pub model_lineage_id: String,
    pub representation: ModelRepresentation,
    pub quantization: Option<QuantizationKind>,
    pub artifact_hash: Hash256,
}

pub enum ModelRepresentation {
    Safetensors,
    Gguf,
}

pub enum QuantizationKind {
    F16,
    Bf16,
    Q8_0,
    Q6_K,
    Q4_K_M,
}

pub struct SubstrateManifest {
    pub schema_version: u32,
    pub model: ModelIdentity,
    pub architecture: ArchitectureFingerprint,
    pub config_hash: Hash256,
    pub tokenizer_hash: Hash256,
    pub index_hash: Option<Hash256>,
    pub tensor_count: u64,
    pub parameter_count: u64,
    pub shards: Vec<ShardManifest>,
    pub tensors: Vec<TensorDescriptor>,
}
```

Hashing MUST be SHA-256 or a stronger explicitly versioned alternative. Do not use process-local/non-cryptographic hash functions for artifact identity.

---

# 2. Canonical Qwen3.8-27B import

Implement an LMML import path for:

```text
/repos/qwen38-safetensors/
```

Suggested CLI shape (adapt to existing LMML conventions rather than forcing these names):

```bash
lmml model import /repos/qwen38-safetensors/
lmml model verify <model-id>
lmml model inspect <model-id>
```

Import must:

1. locate and validate `config.json`;
2. locate tokenizer metadata/artifacts used by the checkpoint;
3. locate `model.safetensors.index.json` when present;
4. enumerate every referenced `.safetensors` shard;
5. verify every referenced shard exists;
6. hash config/tokenizer/index/shards;
7. enumerate tensor names, dtypes and shapes from Safetensors metadata;
8. calculate parameter count from actual tensor shapes;
9. discover architecture fields from the ACTUAL Qwen3.8 configuration;
10. emit a deterministic canonical manifest;
11. refuse stale or inconsistent mixed-model directories.

Do NOT hard-code:
- block count;
- hidden size;
- FFN width;
- vocabulary size;
- attention/recurrent layer pattern;
- tensor naming;
- tap layer positions.

The actual checkpoint is authoritative.

---

# 3. Model artifact DAG

Add a persistent model-artifact lineage graph.

Conceptual relationship:

```text
Qwen3.8-27B Safetensors
        |
        +--> BF16/F16 GGUF
        |
        +--> Q8_0 GGUF
        |
        +--> Q6_K GGUF
        |
        +--> Q4_K_M GGUF
        |
        +--> LoRA adapter A1
                 |
                 +--> merged successor M1 Safetensors
                           |
                           +--> M1 Q8_0
                           +--> M1 Q6_K
                           +--> M1 Q4_K_M
```

Each artifact record must include:

```rust
pub struct ArtifactManifest {
    pub schema_version: u32,
    pub artifact: ArtifactIdentity,
    pub parent_artifact: Option<String>,
    pub canonical_model: String,

    pub tool: String,
    pub tool_version: String,
    pub command_or_parameters: Vec<String>,

    pub source_hashes: Vec<Hash256>,
    pub output_hash: Hash256,

    pub created_at: String,
}
```

Quantization and conversion MUST create a new artifact; they never mutate the parent.

---

# 4. GGUF conversion and quantization

Integrate the existing llama.cpp submodule/tooling as the initial deployment conversion backend.

Required derived variants:

```text
unquantized/near-master GGUF: F16 or BF16 where supported
Q8_0
Q6_K
Q4_K_M
```

If a requested quantization is unsupported by the installed llama.cpp revision:
- return an explicit unsupported error;
- do not silently substitute another quantization.

Suggested LMML command surface:

```bash
lmml model derive <model-id> --format gguf --dtype f16
lmml model derive <model-id> --format gguf --quant q8_0
lmml model derive <model-id> --format gguf --quant q6_k
lmml model derive <model-id> --format gguf --quant q4_k_m
```

The implementation may call llama.cpp conversion/quantization utilities, but LMML owns:
- preflight verification;
- process invocation;
- output path allocation;
- artifact manifest;
- hashing;
- lineage;
- post-conversion verification.

Post-conversion gate:
- file exists;
- GGUF metadata can be inspected;
- model lineage metadata is recorded;
- SHA-256 computed;
- backend can at least open/inspect the artifact before it is marked usable.

Never delete or replace `/repos/qwen38-safetensors/`.

---

# 5. Runtime capability model

Add a backend-neutral model request/lease contract.

Conceptual types:

```rust
pub struct ModelRequest {
    pub model_lineage_id: String,
    pub purpose: InferencePurpose,
    pub representation_preference: Vec<ModelRepresentationPreference>,
    pub required_capabilities: Vec<RuntimeCapability>,
}

pub enum RuntimeCapability {
    TextGeneration,
    Logits,
    Embeddings,
    HiddenStateObservation,
    ElevenTapObservation,
}

pub struct ModelLease {
    pub lease_id: String,
    pub model_lineage_id: String,
    pub artifact_id: String,
    pub runtime_id: String,
    pub endpoint: RuntimeEndpoint,
    pub manifest_hash: Hash256,
    pub artifact_hash: Hash256,
    pub capabilities: Vec<RuntimeCapability>,
}
```

A caller such as AgentQ/CROWN should request a capability, not a filesystem path.

LMML selects:
- artifact;
- quantization;
- device/backend;
- runtime endpoint.

---

# 6. llama.cpp deployment backend

Use llama.cpp as the default GGUF deployment backend.

Preserve existing LMML launch semantics and configuration where possible.

Support at minimum:
- explicit model artifact selection;
- context size;
- GPU/offload settings already available in LMML;
- current mmap behavior;
- current KV-cache settings/config passthrough;
- health/liveness;
- runtime manifest reporting.

The runtime record must state the exact artifact being executed:

```rust
pub struct RuntimeManifest {
    pub runtime_id: String,
    pub model_lineage_id: String,
    pub artifact_id: String,
    pub artifact_hash: Hash256,
    pub representation: ModelRepresentation,
    pub quantization: Option<QuantizationKind>,
    pub backend: String,
    pub backend_version: String,
    pub context_size: usize,
    pub capabilities: Vec<RuntimeCapability>,
}
```

Do not infer canonical model identity from the GGUF file alone; link it to the canonical Safetensors lineage.

---

# 7. Research/instrumented runtime capability

CROWN11 will eventually require real intermediate activations.

Do NOT pretend ordinary llama-server provides arbitrary-layer hidden states if it does not.

Add an abstraction:

```rust
pub trait ActivationObservationProvider {
    fn capabilities(&self) -> &[RuntimeCapability];

    fn observe(
        &self,
        request: ActivationObservationRequest,
    ) -> Result<ActivationObservationResponse, ActivationObservationError>;
}
```

LMML must be able to represent:

```text
llama.cpp runtime:
    TextGeneration = yes
    Logits = yes/if actually available through current integration
    ElevenTapObservation = no unless explicitly implemented

instrumented research runtime:
    ElevenTapObservation = yes only when a real provider is installed
```

For the initial update, it is acceptable to implement the capability plumbing plus an explicit `Unsupported` response for llama.cpp.

If an instrumented Transformers/PyTorch sidecar already exists or is added:
- LMML may manage its process lifecycle;
- it must identify the same canonical model lineage;
- raw activations remain observations, not semantic meaning.

No fabricated tap data.

---

# 8. Baseline inference gate

Before any CROWN-specific training or model mutation, create a frozen pristine baseline.

For at least a small deterministic evaluation set:
- fixed prompts/input IDs;
- fixed generation/logit settings;
- model artifact recorded;
- backend version recorded;
- output/logit hashes recorded where practical.

Produce:

```text
baseline/qwen38-27b/<baseline-id>/
```

with a manifest.

This baseline becomes the anchor for later successor regression.

The test must prove the model can run through LMML with no CROWN/tap adapter attached.

---

# 9. Successor fine-tuning pipeline

Implement the deferred successor-training hardening.

The successor pipeline is:

```text
M0 canonical Safetensors
 -> TRAIN
 -> adapter candidate A*
 -> adapter validation
 -> MERGE
 -> merged candidate M1*
 -> structural + numerical + regression admission
 -> ADMIT M1
 -> optional GGUF/quantization derivatives
```

Hard laws:

```text
TRAIN != MERGE
MERGE != ADMIT
ADMIT != QUANTIZE
failed admission => M0 untouched
```

## 9.1 Base identity gate

Every training run records:
- canonical base model lineage ID;
- canonical manifest hash;
- tokenizer hash;
- config hash;
- training dataset identity/hash;
- seed;
- training config.

Reject a run if the selected base does not match the requested canonical substrate.

## 9.2 Trainable parameter allowlist

Enumerate every trainable parameter before the first optimization step.

Require:

```text
Trainable(theta) subset_of approved_allowlist
```

Initially the allowlist should cover only explicitly configured adapters/LoRA modules.

Fail on an unexpected trainable base parameter.

## 9.3 Finite-state gates

Before writing a checkpoint:
- loss finite;
- tracked gradient norms finite;
- saved adapter tensors finite.

NaN/Inf => hard failure.

## 9.4 Safetensors adapter gate

Validate produced adapter Safetensors:
- readable;
- expected names;
- expected shapes;
- expected dtypes;
- finite tensors;
- no unintended full base weights;
- deterministic hash/manifest.

## 9.5 `merge_lora.py` dtype fix

Update the merge path so model loading uses an explicit supported `dtype=` argument where required by the current Transformers API.

Do not rely on deprecated or ambiguous dtype behavior.

The merge contract should record:
- requested dtype;
- effective load dtype;
- merge dtype;
- output dtype.

Canonical successor merge should occur from an unquantized/appropriate-precision representation, not from a Q4 deployment artifact.

## 9.6 Candidate-only merge

Write merged output to a NEW directory.

Never overwrite the parent.

Conceptual:

```text
successors/<candidate-id>/
```

## 9.7 Merged Safetensors structural validation

After merge:
- all expected tensors present;
- no unexpected architecture mutation;
- expected shapes;
- finite values;
- output manifest + hash.

For a normal LoRA merge, base architecture/tensor shapes should remain structurally compatible.

## 9.8 Prove the merge changed something

Compute/report at least:
- changed tensor count;
- unchanged tensor count;
- max absolute delta or norm;
- aggregate norm delta.

Require at least one intended tensor to have a non-trivial delta.

A no-op merge is a failed merge.

## 9.9 Adapter-vs-merged equivalence

For deterministic sample inputs, compare:

```text
base + live adapter
vs
merged successor
```

Prefer logits rather than sampled generation.

Require a configured numerical tolerance.

If exact framework support is unavailable, build this gate in the training-side Python environment rather than faking it in Rust.

## 9.10 Regression/anchor gate

Use the frozen Qwen3.8 baseline suite.

The candidate successor must stay within configured degradation bounds on the anchor set.

Record:
- parent baseline metrics;
- candidate metrics;
- deltas;
- pass/fail threshold.

## 9.11 Admission

Only after all gates pass create:

```text
successor_manifest.json
```

containing:
- parent model;
- adapter;
- dataset;
- training config;
- seed;
- allowlist;
- dtype;
- merge tool/version;
- tensor manifest;
- equivalence result;
- regression result;
- successor hash.

Then and only then mark the successor as admitted.

---

# 10. Quantize only admitted successors

After `M1` is admitted, allow:

```text
M1 Safetensors
 -> M1 GGUF F16/BF16
 -> M1 Q8_0
 -> M1 Q6_K
 -> M1 Q4_K_M
```

Each derivative preserves the parent successor lineage ID.

Do not quantize an unadmitted candidate into a production artifact by default.

---

# 11. LMML API/CLI additions

Adapt names to current CLI conventions, but provide equivalent operations:

```text
model import
model inspect
model verify
model lineage
model derive
model artifacts
model baseline
model train-successor
model merge-successor
model admit-successor
runtime request/lease
runtime inspect
runtime stop
```

All commands must be scriptable/non-interactive.

Machine-readable JSON output should be available for AgentQ/CROWN integration.

---

# 12. Storage layout

Do not hard-code everything into source, but use a coherent managed layout.

Example:

```text
<LMML_STATE>/models/
  lineage/
  manifests/
  artifacts/
    gguf/
    adapters/
    successors/
  baselines/
  runtimes/
```

The external canonical source may remain:

```text
/repos/qwen38-safetensors/
```

LMML may reference or import it according to current LMML storage semantics, but must never destructively rewrite it.

---

# 13. Error model

Prefer local typed errors rather than broad catch-all strings.

Required distinctions include:
- canonical source missing;
- shard missing;
- manifest mismatch;
- mixed/stale model artifact;
- unsupported quantization;
- converter failed;
- artifact verification failed;
- runtime capability unsupported;
- base lineage mismatch;
- unexpected trainable parameter;
- non-finite training state;
- no-op merge;
- equivalence failure;
- regression gate failure.

Do not silently downgrade failures.

---

# 14. Tests

Add unit/integration tests for at least:

1. canonical manifest stable for same files;
2. missing shard rejected;
3. changed shard changes manifest hash;
4. stale Qwen3.5 artifact cannot satisfy Qwen3.8 request;
5. ModelIdentity differs from ArtifactIdentity;
6. GGUF derivative links to canonical parent;
7. unsupported quantization refuses;
8. runtime lease reports exact artifact hash;
9. llama.cpp backend does not claim ElevenTapObservation unless implemented;
10. base checkpoint path is never overwritten by derive/merge;
11. unexpected trainable parameter rejected;
12. NaN/Inf adapter rejected;
13. no-op merge rejected;
14. successor requires explicit parent;
15. unadmitted successor is not selected as production default;
16. admitted successor can produce GGUF derivatives;
17. baseline regression metadata is persisted.

Do not mark work complete unless full LMML test suite is green.

---

# 15. Deliverables

Return:

1. concise architecture summary;
2. exact files changed/added;
3. new CLI/API surface;
4. manifest/schema examples;
5. model lineage example from Safetensors -> Q8/Q6/Q4;
6. runtime lease example;
7. successor lineage example;
8. all tests run and exact pass/fail totals;
9. warnings, clearly separating pre-existing from new;
10. any blocked step requiring actual Qwen3.8 files/tools;
11. no claim of hidden-state/tap support unless demonstrated.

---

# 16. Definition of done

This LMML increment is complete when:

```text
/repos/qwen38-safetensors or configured canonical path
 -> deterministic canonical manifest
 -> LMML ModelIdentity
 -> GGUF F16/BF16 derivative
 -> Q8_0 / Q6_K / Q4_K_M derivatives
 -> artifact DAG
 -> llama.cpp runtime lease
 -> pristine baseline
```

works with verified lineage, and the successor training/merge/admission path enforces the checkpoint gates above.

No CROWN policy semantics are moved into LMML.
No learned tap weights are fabricated.
No stale Qwen3.5-9B artifact is allowed to masquerade as Qwen3.8-27B.
