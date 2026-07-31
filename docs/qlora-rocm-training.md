# ROCm QLoRA Adapter Training

This guide covers an LMML-adjacent QLoRA workflow for external chat JSONL
adapters on AMD ROCm systems such as the Radeon AI PRO R9700.

## Boundary

- LMML owns local `llama.cpp` build, GGUF serving, adapter conversion, and the
  wrapper scripts in this repository.
- PyTorch/PEFT owns QLoRA tensor training inside a ROCm container.
- External model shards and training data stay outside this repository and are
  mounted read-only.
- The current stock `llama-finetune` path remains full-model GGUF fine-tuning;
  do not use it for this adapter run unless it advertises `--lora-out`.

## Host Paths

The wrapper expects a separate training-source directory. Set
`TRAINING_SOURCE_ROOT` when your files live elsewhere:

```text
$TRAINING_SOURCE_ROOT/safe_tensors
$TRAINING_SOURCE_ROOT/data/train.jsonl
$HOME/.local/share/lmml/llama.cpp
./outputs/qlora
```

The matching GGUF files are used after adapter training:

```text
$HOME/.local/share/lmml/models/Qwen3.5-27B-BF16.gguf
$HOME/.local/share/lmml/models/Qwen3.5-27B-Q6_K.gguf
```

## Docker Image

Use AMD's validated ROCm PyTorch image by default:

```sh
docker pull rocm/pytorch:rocm7.2.4_ubuntu24.04_py3.12_pytorch_release_2.9.1
```

The wrapper uses the same device shape as AMD's documented launch pattern:
`/dev/kfd`, `/dev/dri`, `SYS_PTRACE`, relaxed seccomp, host IPC, and video/render
groups.

Python adapter-training dependencies are installed into a persistent host-mounted
target:

```text
~/.local/share/lmml/qlora-python
```

The Docker container itself is disposable, so installs made into the container's
system Python do not survive `docker run --rm`.

The wrapper installs only adapter-specific Python packages into that target and
intentionally does not install `torch`, `torchvision`, `torchaudio`, `triton`, or
NVIDIA CUDA wheels. Those would shadow the ROCm PyTorch stack already inside the
AMD image.

## Setup Check

```sh
scripts/qlora-rocm-docker.sh doctor
```

If an earlier run installed PyPI `torch`, NVIDIA CUDA packages, or incompatible
dependency versions into the persistent dependency target, reset it before
retrying:

```sh
scripts/qlora-rocm-docker.sh reset-deps
scripts/qlora-rocm-docker.sh prepare
```

After dependencies have been installed once into the persistent target, you can
skip the dependency check/install step:

```sh
LMML_QLORA_INSTALL_DEPS=0 scripts/qlora-rocm-docker.sh doctor
```

If Docker reports a socket permission error, add the user to the Docker group and
start a new login session:

```sh
sudo usermod -aG docker "$USER"
```

## Dataset And Tokenizer Check

This validates the tokenizer, chat-template rendering, label masking, and
manifest writing without loading the 27B model. The trainer uses a simple
in-memory dataset for small and medium JSONL adapter runs, avoiding Hugging Face
`datasets`/pyarrow fingerprinting issues.

```sh
scripts/qlora-rocm-docker.sh prepare
```

## Smoke Adapter Run

Start with a short, conservative run:

```sh
scripts/qlora-rocm-docker.sh smoke
```

The smoke defaults are:

```text
OUT_DIR=/workspace/lmml/outputs/qlora/qwen35-27b-r9700-qlora-smoke
MAX_STEPS=20
MAX_SAMPLES=256
SEQ_LEN=512
LORA_R=8
GRAD_ACCUM=8
LR=1e-5
COMPUTE_DTYPE=auto
OPTIM=adamw_torch
EMPTY_CACHE_STEPS=0
LOG_MEMORY_STEPS=1
SAVE_STRATEGY=steps
SAVE_STEPS=5
SAVE_TOTAL_LIMIT=2
TRACE_BATCH_ROWS=1
SUSPECT_ROWS=42,178
ONLY_SOURCE_LINES=
PAD_TO_MULTIPLE_OF=0
SHUFFLE_DATA=1
TRACE_BACKWARD_MODULES=0
BACKWARD_TRACE_MARKERS=Linear4bit,GatedDeltaNet,DeltaNet
TRACE_BACKWARD_PREFIX=
LORA_EXCLUDE_MODULES=
LORA_AUTOCAST_ADAPTER_DTYPE=1
ROCM_BLAS_BACKEND=rocblas
ROCBLAS_USE_HIPBLASLT=0
DATALOADER_NUM_WORKERS=0
DATALOADER_PIN_MEMORY=0
DATALOADER_PERSISTENT_WORKERS=0
ATTN_IMPLEMENTATION=eager
FORCE_MATH_SDP=1
```

The 20-step smoke uses 256 rows by default. With `GRAD_ACCUM=8`, that keeps the
run inside the first dataloader pass and avoids testing epoch-wrap behavior
before the base sustained-training path is validated.

`EMPTY_CACHE_STEPS=0` is intentional. Calling `torch.cuda.empty_cache()` after
every optimizer step creates avoidable HIP allocator churn and can hide the real
fault boundary. Leave it disabled unless testing a specific fragmentation
hypothesis. The smoke path logs ROCm/PyTorch memory counters every optimizer step
and checkpoints adapters every five optimizer steps so diagnostic runs still
produce recoverable artifacts.

`ROCM_BLAS_BACKEND=rocblas` and `ROCBLAS_USE_HIPBLASLT=0` are also intentional
R9700 defaults. A one-row reproducer hit a deterministic GPUVM fault in the
default GEMM path during LoRA-A backward; forcing the rocBLAS/Tensile path
completed forward, backward, optimizer step, and adapter save.

## ROCm Fault Triage

If training dies with a GPUVM fault such as `Page not present or supervisor
privilege`, first split the smoke into three controlled runs:

```sh
sudo env \
  LMML_QLORA_INSTALL_DEPS=0 \
  MAX_SAMPLES=64 \
  MAX_STEPS=8 \
  EMPTY_CACHE_STEPS=0 \
  scripts/qlora-rocm-docker.sh smoke
```

```sh
sudo env \
  LMML_QLORA_INSTALL_DEPS=0 \
  MAX_SAMPLES=64 \
  MAX_STEPS=12 \
  EMPTY_CACHE_STEPS=0 \
  scripts/qlora-rocm-docker.sh smoke
```

```sh
sudo env \
  LMML_QLORA_INSTALL_DEPS=0 \
  MAX_SAMPLES=256 \
  MAX_STEPS=20 \
  EMPTY_CACHE_STEPS=0 \
  scripts/qlora-rocm-docker.sh smoke
```

Interpretation:

- 8 steps passes and 12 steps fails near step 9: investigate dataloader wrap or
  epoch-boundary behavior.
- 12 steps and 20 steps both fail near the same sustained step count:
  investigate HIP kernels, allocator/runtime behavior, or driver reset.
- 12 steps passes after disabling `empty_cache()`: allocator churn was likely
  involved.
- Failure moves randomly: treat host driver/kernel stability as the primary
  suspect.

For a failing case, force more synchronous HIP reporting:

```sh
sudo env \
  LMML_QLORA_INSTALL_DEPS=0 \
  MAX_SAMPLES=64 \
  MAX_STEPS=12 \
  EMPTY_CACHE_STEPS=0 \
  HIP_LAUNCH_BLOCKING=1 \
  AMD_SERIALIZE_KERNEL=3 \
  AMD_SERIALIZE_COPY=3 \
  TORCH_SHOW_CPP_STACKTRACES=1 \
  PYTHONFAULTHANDLER=1 \
  scripts/qlora-rocm-docker.sh smoke \
  2>&1 | tee qlora-rocm-sync-debug.log
```

Only after that, test with PyTorch's HIP cache disabled:

```sh
sudo env \
  LMML_QLORA_INSTALL_DEPS=0 \
  MAX_SAMPLES=64 \
  MAX_STEPS=12 \
  EMPTY_CACHE_STEPS=0 \
  HIP_LAUNCH_BLOCKING=1 \
  AMD_SERIALIZE_KERNEL=3 \
  AMD_SERIALIZE_COPY=3 \
  PYTORCH_NO_HIP_MEMORY_CACHING=1 \
  TORCH_SHOW_CPP_STACKTRACES=1 \
  PYTHONFAULTHANDLER=1 \
  scripts/qlora-rocm-docker.sh smoke \
  2>&1 | tee qlora-rocm-no-cache-debug.log
```

Use the matrix mode to capture the exact container software stack:

```sh
sudo env LMML_QLORA_INSTALL_DEPS=0 scripts/qlora-rocm-docker.sh matrix
```

Capture host driver evidence from another terminal while the failing run is
active:

```sh
sudo journalctl -kf |
  grep --line-buffered -Ei \
  'amdgpu|kfd|gpu reset|vm fault|page fault|ring timeout|MES|gfxhub|mmhub'
```

After a crash:

```sh
sudo dmesg -T |
  grep -Ei \
  'amdgpu|kfd|gpu reset|vm fault|page fault|ring timeout|gfxhub|mmhub' |
  tail -n 250
```

The post-crash `amd-smi` idle VRAM number is not the training peak; use the
`[rocm-memory]` trainer lines for the high-water mark.

If the 256-row smoke fails before epoch wrap, remove checkpoints and callbacks:

```sh
sudo env \
  LMML_QLORA_INSTALL_DEPS=0 \
  MAX_SAMPLES=256 \
  MAX_STEPS=8 \
  EMPTY_CACHE_STEPS=0 \
  LOG_MEMORY_STEPS=0 \
  SAVE_STRATEGY=no \
  TRACE_BATCH_ROWS=1 \
  scripts/qlora-rocm-docker.sh smoke
```

Interpretation:

- Fails around the same step: checkpointing and memory callbacks are not needed
  to trigger the fault.
- Completes: checkpoint saving or callback activity is interacting with the GPU
  runtime.
- Fails at a different step: asynchronous kernel corruption or runtime timing is
  more likely than Trainer control flow.

Then force synchronous HIP reporting on the failing case. Six optimizer steps is
enough when the fault happens during accumulation for step 6:

```sh
sudo env \
  LMML_QLORA_INSTALL_DEPS=0 \
  MAX_SAMPLES=256 \
  MAX_STEPS=6 \
  EMPTY_CACHE_STEPS=0 \
  LOG_MEMORY_STEPS=0 \
  SAVE_STRATEGY=no \
  TRACE_BATCH_ROWS=1 \
  HIP_LAUNCH_BLOCKING=1 \
  AMD_SERIALIZE_KERNEL=3 \
  AMD_SERIALIZE_COPY=3 \
  TORCH_SHOW_CPP_STACKTRACES=1 \
  PYTHONFAULTHANDLER=1 \
  scripts/qlora-rocm-docker.sh smoke \
  2>&1 | tee qlora-r9700-sync-step6.log
```

Smoke mode prints `[forward-enter]`, `[forward-exit]`, `[backward-enter]`, and
`[backward-exit]` row IDs and source JSONL lines around each traced microbatch.
It also synchronizes before forward, after forward, and after backward, so the
last line identifies whether the fault came from the current row's forward path,
backward path, or later Trainer control flow.

## Direct Source-Line Replay

Once a source line is identified, replay it directly. `ONLY_SOURCE_LINES` is
applied while reading the JSONL file, so `MAX_SAMPLES=1` means one matched source
line rather than the first line in the file.

Replay source line 179 unchanged:

```sh
sudo env \
  LMML_QLORA_INSTALL_DEPS=0 \
  ONLY_SOURCE_LINES=179 \
  MAX_SAMPLES=1 \
  MAX_STEPS=1 \
  GRAD_ACCUM=1 \
  EMPTY_CACHE_STEPS=0 \
  LOG_MEMORY_STEPS=0 \
  SAVE_STRATEGY=no \
  TRACE_BATCH_ROWS=1 \
  PAD_TO_MULTIPLE_OF=0 \
  HIP_LAUNCH_BLOCKING=1 \
  AMD_SERIALIZE_KERNEL=3 \
  AMD_SERIALIZE_COPY=3 \
  PYTHONFAULTHANDLER=1 \
  scripts/qlora-rocm-docker.sh smoke \
  2>&1 | tee row179-direct.log
```

Replay source line 43 as the known-good control:

```sh
sudo env \
  LMML_QLORA_INSTALL_DEPS=0 \
  ONLY_SOURCE_LINES=43 \
  MAX_SAMPLES=1 \
  MAX_STEPS=1 \
  GRAD_ACCUM=1 \
  EMPTY_CACHE_STEPS=0 \
  LOG_MEMORY_STEPS=0 \
  SAVE_STRATEGY=no \
  TRACE_BATCH_ROWS=1 \
  PAD_TO_MULTIPLE_OF=0 \
  HIP_LAUNCH_BLOCKING=1 \
  AMD_SERIALIZE_KERNEL=3 \
  AMD_SERIALIZE_COPY=3 \
  PYTHONFAULTHANDLER=1 \
  scripts/qlora-rocm-docker.sh smoke \
  2>&1 | tee row43-control.log
```

Replay source line 179 with dynamic padding aligned to 8:

```sh
sudo env \
  LMML_QLORA_INSTALL_DEPS=0 \
  ONLY_SOURCE_LINES=179 \
  MAX_SAMPLES=1 \
  MAX_STEPS=1 \
  GRAD_ACCUM=1 \
  EMPTY_CACHE_STEPS=0 \
  LOG_MEMORY_STEPS=0 \
  SAVE_STRATEGY=no \
  TRACE_BATCH_ROWS=1 \
  PAD_TO_MULTIPLE_OF=8 \
  HIP_LAUNCH_BLOCKING=1 \
  AMD_SERIALIZE_KERNEL=3 \
  AMD_SERIALIZE_COPY=3 \
  PYTHONFAULTHANDLER=1 \
  scripts/qlora-rocm-docker.sh smoke \
  2>&1 | tee row179-pad8.log
```

Interpretation:

- Row 179 fails with `PAD_TO_MULTIPLE_OF=0` and passes with `8`: shape/alignment
  sensitive GPU kernel bug.
- Row 179 fails both ways: content pattern or general backward-kernel bug.
- Row 179 passes alone: the failure requires accumulation history or the
  preceding shape sequence.
- Row 43 passes and row 179 fails: the trigger remains isolated to line 179.

If row 179 passes alone, replay the known transition in source order:

```sh
sudo env \
  LMML_QLORA_INSTALL_DEPS=0 \
  ONLY_SOURCE_LINES=43,179 \
  MAX_SAMPLES=2 \
  MAX_STEPS=1 \
  GRAD_ACCUM=2 \
  SHUFFLE_DATA=0 \
  PAD_TO_MULTIPLE_OF=0 \
  EMPTY_CACHE_STEPS=0 \
  LOG_MEMORY_STEPS=0 \
  SAVE_STRATEGY=no \
  TRACE_BATCH_ROWS=1 \
  HIP_LAUNCH_BLOCKING=1 \
  AMD_SERIALIZE_KERNEL=3 \
  AMD_SERIALIZE_COPY=3 \
  PYTHONFAULTHANDLER=1 \
  scripts/qlora-rocm-docker.sh smoke \
  2>&1 | tee row43-row179-transition.log
```

If row 179 still fails alone, trace the suspicious backward module families:

```sh
sudo env \
  LMML_QLORA_INSTALL_DEPS=0 \
  ONLY_SOURCE_LINES=179 \
  MAX_SAMPLES=1 \
  MAX_STEPS=1 \
  GRAD_ACCUM=1 \
  EMPTY_CACHE_STEPS=0 \
  LOG_MEMORY_STEPS=0 \
  SAVE_STRATEGY=no \
  TRACE_BATCH_ROWS=1 \
  TRACE_BACKWARD_MODULES=1 \
  BACKWARD_TRACE_MARKERS=Linear4bit,GatedDeltaNet,DeltaNet \
  PAD_TO_MULTIPLE_OF=0 \
  HIP_LAUNCH_BLOCKING=1 \
  AMD_SERIALIZE_KERNEL=3 \
  AMD_SERIALIZE_COPY=3 \
  PYTHONFAULTHANDLER=1 \
  TORCH_DISABLE_ADDR2LINE=1 \
  scripts/qlora-rocm-docker.sh smoke \
  2>&1 | tee row179-backward-modules.log
```

The last `[backward-module-enter]` without a matching
`[backward-module-exit]` identifies the module family that launched the failing
backward path.

After the broad trace identifies a specific projection, trace only that subtree.
This includes the PEFT wrapper, bitsandbytes base layer, LoRA dropout, and LoRA
A/B child modules without installing hooks across the whole 27B graph:

```sh
sudo env \
  LMML_QLORA_INSTALL_DEPS=0 \
  ONLY_SOURCE_LINES=179 \
  MAX_SAMPLES=1 \
  MAX_STEPS=1 \
  GRAD_ACCUM=1 \
  EMPTY_CACHE_STEPS=0 \
  LOG_MEMORY_STEPS=0 \
  SAVE_STRATEGY=no \
  TRACE_BATCH_ROWS=1 \
  TRACE_BACKWARD_MODULES=0 \
  TRACE_BACKWARD_PREFIX=base_model.model.model.layers.4.linear_attn.out_proj \
  PAD_TO_MULTIPLE_OF=0 \
  HIP_LAUNCH_BLOCKING=1 \
  AMD_SERIALIZE_KERNEL=3 \
  AMD_SERIALIZE_COPY=3 \
  PYTHONFAULTHANDLER=1 \
  TORCH_DISABLE_ADDR2LINE=1 \
  scripts/qlora-rocm-docker.sh smoke \
  2>&1 | tee row179-layer4-outproj-subtree.log
```

The last `[subtree-backward-enter]` without a matching
`[subtree-backward-exit]` identifies whether the fault is in the parent LoRA
wrapper, `base_layer`, `lora_A`, `lora_B`, or dropout.

Then test whether removing LoRA from that exact projection avoids the fault:

```sh
sudo env \
  LMML_QLORA_INSTALL_DEPS=0 \
  ONLY_SOURCE_LINES=179 \
  MAX_SAMPLES=1 \
  MAX_STEPS=1 \
  GRAD_ACCUM=1 \
  EMPTY_CACHE_STEPS=0 \
  LOG_MEMORY_STEPS=0 \
  SAVE_STRATEGY=no \
  TRACE_BATCH_ROWS=1 \
  TRACE_BACKWARD_MODULES=0 \
  LORA_EXCLUDE_MODULES=layers.4.linear_attn.out_proj \
  PAD_TO_MULTIPLE_OF=0 \
  HIP_LAUNCH_BLOCKING=1 \
  AMD_SERIALIZE_KERNEL=3 \
  AMD_SERIALIZE_COPY=3 \
  PYTHONFAULTHANDLER=1 \
  scripts/qlora-rocm-docker.sh smoke \
  2>&1 | tee row179-exclude-layer4-outproj.log
```

If the exact exclusion passes, broaden the production workaround cautiously:
`LORA_EXCLUDE_MODULES=regex:.*linear_attn\.out_proj$`.

If excluding one adapter moves the fault instead of clearing it, switch from
module-local isolation to ROCm GEMM backend isolation. The wrapper already
defaults to PyTorch's rocBLAS-compatible selector and asks rocBLAS not to prefer
hipBLASLt, but keep the values explicit in one-line reproductions:

```sh
sudo env \
  LMML_QLORA_INSTALL_DEPS=0 \
  ONLY_SOURCE_LINES=179 \
  MAX_SAMPLES=1 \
  MAX_STEPS=1 \
  GRAD_ACCUM=1 \
  EMPTY_CACHE_STEPS=0 \
  LOG_MEMORY_STEPS=0 \
  SAVE_STRATEGY=no \
  TRACE_BATCH_ROWS=1 \
  TRACE_BACKWARD_MODULES=0 \
  LORA_EXCLUDE_MODULES= \
  ROCM_BLAS_BACKEND=rocblas \
  ROCBLAS_USE_HIPBLASLT=0 \
  HIP_LAUNCH_BLOCKING=1 \
  AMD_SERIALIZE_KERNEL=3 \
  AMD_SERIALIZE_COPY=3 \
  PYTHONFAULTHANDLER=1 \
  scripts/qlora-rocm-docker.sh smoke \
  2>&1 | tee row179-force-rocblas.log
```

The trainer prints
`[runtime] requested_rocm_blas=... pytorch_blas_backend=... ROCBLAS_USE_HIPBLASLT=...`
when this selector is active. On ROCm builds, PyTorch's historical `Cublas`
backend label is the rocBLAS-compatible path.

If forcing rocBLAS still fails, explicitly test the hipBLASLt path:

```sh
sudo env \
  LMML_QLORA_INSTALL_DEPS=0 \
  ONLY_SOURCE_LINES=179 \
  MAX_SAMPLES=1 \
  MAX_STEPS=1 \
  GRAD_ACCUM=1 \
  EMPTY_CACHE_STEPS=0 \
  LOG_MEMORY_STEPS=0 \
  SAVE_STRATEGY=no \
  TRACE_BATCH_ROWS=1 \
  TRACE_BACKWARD_MODULES=0 \
  LORA_EXCLUDE_MODULES= \
  ROCM_BLAS_BACKEND=hipblaslt \
  ROCBLAS_USE_HIPBLASLT=1 \
  HIP_LAUNCH_BLOCKING=1 \
  AMD_SERIALIZE_KERNEL=3 \
  AMD_SERIALIZE_COPY=3 \
  PYTHONFAULTHANDLER=1 \
  scripts/qlora-rocm-docker.sh smoke \
  2>&1 | tee row179-force-hipblaslt.log
```

Then test whether PEFT's adapter dtype promotion is involved. By default PEFT may
autocast adapter weights; disabling it keeps the adapter in the lower-precision
path where supported:

```sh
sudo env \
  LMML_QLORA_INSTALL_DEPS=0 \
  ONLY_SOURCE_LINES=179 \
  MAX_SAMPLES=1 \
  MAX_STEPS=1 \
  GRAD_ACCUM=1 \
  EMPTY_CACHE_STEPS=0 \
  LOG_MEMORY_STEPS=0 \
  SAVE_STRATEGY=no \
  TRACE_BATCH_ROWS=1 \
  TRACE_BACKWARD_MODULES=0 \
  LORA_AUTOCAST_ADAPTER_DTYPE=0 \
  ROCM_BLAS_BACKEND=rocblas \
  ROCBLAS_USE_HIPBLASLT=0 \
  HIP_LAUNCH_BLOCKING=1 \
  AMD_SERIALIZE_KERNEL=3 \
  AMD_SERIALIZE_COPY=3 \
  PYTHONFAULTHANDLER=1 \
  scripts/qlora-rocm-docker.sh smoke \
  2>&1 | tee row179-rocblas-bf16-lora.log
```

The trainer prints `[lora-dtype]` for
`layers.4.linear_attn.out_proj.lora_A.default`, which confirms the adapter dtype
used in the failing subtree probe.

After the one-row rocBLAS replay passes, validate the original smoke shape with
debug synchronization removed:

```sh
sudo env \
  LMML_QLORA_INSTALL_DEPS=0 \
  MAX_SAMPLES=256 \
  MAX_STEPS=20 \
  GRAD_ACCUM=8 \
  EMPTY_CACHE_STEPS=0 \
  LOG_MEMORY_STEPS=1 \
  SAVE_STRATEGY=no \
  TRACE_BATCH_ROWS=0 \
  TRACE_BACKWARD_MODULES=0 \
  LORA_EXCLUDE_MODULES= \
  ROCM_BLAS_BACKEND=rocblas \
  ROCBLAS_USE_HIPBLASLT=0 \
  scripts/qlora-rocm-docker.sh smoke \
  2>&1 | tee qlora-r9700-rocblas-20step.log
```

Then repeat with 50 and 100 steps before restoring checkpointing.

If no-save completes, stress checkpointing deliberately:

```sh
sudo env \
  LMML_QLORA_INSTALL_DEPS=0 \
  MAX_SAMPLES=64 \
  MAX_STEPS=8 \
  EMPTY_CACHE_STEPS=0 \
  LOG_MEMORY_STEPS=0 \
  SAVE_STRATEGY=steps \
  SAVE_STEPS=1 \
  SAVE_TOTAL_LIMIT=1 \
  TRACE_BATCH_ROWS=1 \
  scripts/qlora-rocm-docker.sh smoke
```

## Full Adapter Run

After the smoke run succeeds:

```sh
SEQ_LEN=2048 \
LORA_R=16 \
LORA_ALPHA=32 \
GRAD_ACCUM=16 \
NUM_EPOCHS=1 \
LR=1e-5 \
scripts/qlora-rocm-docker.sh train
```

Increase `SEQ_LEN` only after measuring VRAM. Do not start with 256k context
training.

If ROCm remains stable and you want to experiment with lower optimizer memory,
try the bitsandbytes paged optimizer explicitly:

```sh
OPTIM=paged_adamw_8bit scripts/qlora-rocm-docker.sh smoke
```

If PyTorch reports an AOTriton efficient-attention GPU memory access fault, keep
`ATTN_IMPLEMENTATION=eager` and `FORCE_MATH_SDP=1`. Those are the wrapper
defaults because they trade speed for a more conservative ROCm path. Qwen3.5's
Gated DeltaNet linear-attention layers still use their own fast/fallback path;
eager SDPA does not disable that subsystem.

## Convert Adapter To GGUF

```sh
scripts/qlora-rocm-docker.sh convert
```

The converter writes:

```text
outputs/qlora/qwen35-27b-r9700-qlora.gguf
```

## Serve With LMML/llama.cpp

Use the Q6_K deployment model plus the adapter:

```sh
${HOME}/.local/share/lmml/llama.cpp/build/bin/llama-server \
  --model ${HOME}/.local/share/lmml/models/Qwen3.5-27B-Q6_K.gguf \
  --lora ./outputs/qlora/qwen35-27b-r9700-qlora.gguf \
  --ctx-size 4096 \
  -ngl auto \
  -fa on
```

## Notes

- The script targets language-model adapter modules by default:
  `q_proj,k_proj,v_proj,o_proj,in_proj_a,in_proj_b,in_proj_qkv,in_proj_z,out_proj`.
- Add MLP modules later only if VRAM and quality tests justify it.
- Stop any large LMML serving process before training so the R9700 has enough
  free VRAM.
- Adapter outputs and manifests live under `outputs/`, which is local runtime
  state and should not be committed.
