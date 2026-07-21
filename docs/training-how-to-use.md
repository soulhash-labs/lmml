# lmml Training How-To Use Guide

This guide explains how humans and agents should use lmml for native llama.cpp
GGUF fine-tuning.

Current policy:

- Use llama.cpp's native `llama-finetune` binary through `lmml train`.
- Treat upstream `llama-finetune` as a **full-model GGUF fine-tuner**.
- Do not assume LoRA adapter output unless `llama-finetune --help` explicitly
  advertises custom flags such as `--lora-out`.
- Train from F16/BF16/F32 GGUF models. Do not train from Q8/Q4 deployment
  quantizations.

## Mental Model

```text
raw examples / conversations
        |
        v
plain text training file using target model template
        |
        v
lmml train
        |
        v
llama-finetune --model ... --file ... --output ...
        |
        v
full fine-tuned GGUF
        |
        v
optional llama-quantize for serving
```

For current official upstream llama.cpp, `lmml train` maps friendly lmml flags to
actual binary flags:

| lmml flag | Upstream llama-finetune flag |
| --- | --- |
| `--model-base` | `--model` |
| `--train-data` | `--file` |
| `--output` | `--output` |

lmml detects custom-fork capabilities at runtime by parsing
`llama-finetune --help`. It only passes `--lora-out`, `--checkpoint-in`, or
`--checkpoint-out` when those flags are explicitly advertised.

## Human Workflow

### 1. Verify training binaries

```sh
lmml smoke
/home/user/.local/share/lmml/llama.cpp/build/bin/llama-finetune --help | head
```

Check whether your binary is upstream-style or custom-fork style:

```sh
/home/user/.local/share/lmml/llama.cpp/build/bin/llama-finetune --help | grep -E 'model-base|train-data|lora-out|checkpoint'
```

Expected official upstream behavior: no `--lora-out`.

### 2. Choose the right base model

Use an unquantized or lightly lossy training base:

```text
Good:    Qwen3.5-9B-BF16.gguf
Good:    Qwen3.5-9B-F16.gguf
Possible: F32 GGUF if disk/RAM allow it
Bad:     Qwen3.5-9B-Q8_0.gguf for training
Bad:     Q4_K_M / Q5_K_M / IQ* for training
```

After training, create a deployment quantization from the output GGUF.

### 3. Prepare plain text data

`llama-finetune` consumes a single text stream. It does not understand dataset
columns, JSONL fields, or chat roles unless you render them into text first.

For chat/instruct models, format each example using the model's actual chat
template. For Qwen-style ChatML:

```text
<|im_start|>system
You are a concise local coding assistant.<|im_end|>
<|im_start|>user
Summarize the purpose of lmml in one sentence.<|im_end|>
<|im_start|>assistant
lmml manages local llama.cpp builds, GGUF models, and an OpenAI-compatible server from a TUI.<|im_end|>

<|im_start|>system
You are a concise local coding assistant.<|im_end|>
<|im_start|>user
What should I check before using --lora-out with llama-finetune?<|im_end|>
<|im_start|>assistant
Run llama-finetune --help and only use --lora-out if the installed binary explicitly advertises that flag.<|im_end|>
```

For Llama-style instruct models:

```text
<s>[INST] Summarize the purpose of lmml in one sentence. [/INST] lmml manages local llama.cpp builds, GGUF models, and an OpenAI-compatible server from a TUI.</s>
<s>[INST] What model precision should be used for training? [/INST] Use F16, BF16, or F32 GGUF for training; quantize only after training.</s>
```

For simple completion training:

```text
Question: What does lmml build locally?
Answer: lmml builds llama.cpp locally so GPU backend, CUDA architecture, compiler, and training binaries match the machine.

Question: What endpoint should agents use when the TUI server is ready?
Answer: Agents should use http://127.0.0.1:1200/v1.
```

Save the file:

```sh
mkdir -p data models outputs
$EDITOR data/train.txt
```

### 4. Run upstream full-model fine-tuning

```sh
lmml train \
  --model-base ./models/Qwen3.5-9B-BF16.gguf \
  --train-data ./data/train.txt \
  --output ./outputs/Qwen3.5-9B-lmml-tuned.gguf \
  -- --epochs 3 --ctx-size 512 --batch-size 4 --ubatch-size 4 --n-gpu-layers 32
```

Notes:

- Anything after `--` is passed directly to `llama-finetune`.
- Start with small `--ctx-size`, `--batch-size`, and `--ubatch-size`.
- Increase only after a short smoke run succeeds.
- Use `--n-gpu-layers 0` for CPU-only training.
- For CUDA training, use the highest GPU layer count that fits reliably.

### 5. Quantize the output for serving

```sh
/home/user/.local/share/lmml/llama.cpp/build/bin/llama-quantize \
  ./outputs/Qwen3.5-9B-lmml-tuned.gguf \
  ./outputs/Qwen3.5-9B-lmml-tuned-Q8_0.gguf \
  Q8_0
```

Then select the quantized model in lmml and start the server.

## Agent Workflow

Agents should not guess training flags. They should inspect capabilities first,
write the dataset deterministically, and run short validation jobs before longer
training.

### Agent checklist

1. Check that `llama-finetune` exists and reports help.
2. Detect whether custom adapter flags are available.
3. Confirm the base model is F16/BF16/F32, not Q8/Q4.
4. Render source examples into plain text with the target chat template.
5. Run a tiny smoke training job.
6. Only then run the requested training job.
7. Quantize output for serving if needed.
8. Record exact command, source data path, output model path, and binary version.

### Capability probe

```sh
FINETUNE=/home/user/.local/share/lmml/llama.cpp/build/bin/llama-finetune
$FINETUNE --version
$FINETUNE --help | grep -E 'model-base|train-data|lora-out|checkpoint|--output|--model|--file'
```

Decision rule:

```text
if --lora-out exists:
  custom adapter workflow may be available
else:
  use full-model --output workflow
```

### Dataset rendering rule for agents

Agents must write explicit text, not structured JSON, unless they also render the
JSON into a prompt/completion stream.

Preferred agent-generated training file format:

```text
<|im_start|>system
{stable system instruction}<|im_end|>
<|im_start|>user
{user task}<|im_end|>
<|im_start|>assistant
{ideal assistant answer}<|im_end|>

<|im_start|>system
{stable system instruction}<|im_end|>
<|im_start|>user
{next user task}<|im_end|>
<|im_start|>assistant
{next ideal assistant answer}<|im_end|>
```

Rules:

- Keep role markers exact for the target model.
- Preserve newlines intentionally.
- Avoid trailing commentary outside the template.
- Deduplicate near-identical examples.
- Keep a validation holdout file when possible.
- Do not include secrets from logs, configs, or user machines.

### Agent smoke run

Before a long training job:

```sh
lmml train \
  --model-base ./models/Qwen3.5-9B-BF16.gguf \
  --train-data ./data/train-smoke.txt \
  --output ./outputs/smoke.gguf \
  -- --epochs 1 --ctx-size 256 --batch-size 1 --ubatch-size 1 --n-gpu-layers 0
```

The smoke run is not for quality. It verifies formatting, binary compatibility,
and disk/output behavior.

## Custom LoRA-Capable Fork Workflow

Only use this section when capability detection proves the installed binary
advertises the flags.

```sh
/home/user/.local/share/lmml/llama.cpp/build/bin/llama-finetune --help | grep -E 'lora-out|checkpoint-in|checkpoint-out'
```

If present:

```sh
lmml train \
  --model-base ./models/Qwen3.5-9B-BF16.gguf \
  --train-data ./data/train.txt \
  --lora-out ./outputs/qwen-lmml-adapter.bin \
  --checkpoint-in ./outputs/qwen-lmml-checkpoint.bin \
  --checkpoint-out ./outputs/qwen-lmml-checkpoint.bin \
  -- --epochs 3 --ctx-size 512 --batch-size 4 --n-gpu-layers 32
```

If you also request `--merge-output`, lmml will run `llama-export-lora` after a
successful training command:

```sh
lmml train \
  --model-base ./models/Qwen3.5-9B-BF16.gguf \
  --train-data ./data/train.txt \
  --lora-out ./outputs/qwen-lmml-adapter.bin \
  --merge-output ./outputs/Qwen3.5-9B-lmml-merged.gguf \
  -- --epochs 3 --ctx-size 512 --batch-size 4 --n-gpu-layers 32
```

If the binary does not advertise `--lora-out`, lmml rejects this path before
starting training.

## VRAM And Memory Expectations

Training is much heavier than inference. Treat these as starting points, not
promises.

| Hardware | Model | Suggested starting point |
| --- | --- | --- |
| 11GB GTX 1080 Ti | 4B F16/BF16 | CPU or low GPU layers, `ctx 256-512`, `batch 1`, `ubatch 1` |
| 16GB RTX 5060/5070 Ti | 4B F16/BF16 | partial/full GPU layers, `ctx 512`, `batch 1-4`, `ubatch 1-4` |
| 24GB Quadro M6000 | 4B/9B F16/BF16 | test partial GPU layers first, `ctx 512`, conservative batch |
| CPU-only | small F32/F16 | slow but useful for smoke tests |

Practical rules:

- If CUDA OOM occurs, reduce `--n-gpu-layers`, `--ctx-size`, `--batch-size`, then
  `--ubatch-size`.
- If system RAM or pinned host allocation fails, reduce batch/context and close
  other memory-heavy processes.
- Do not run training while a large-context serving process is active unless the
  machine has clear VRAM/RAM headroom.
- Expect native llama.cpp training to be less optimized than PyTorch/Unsloth for
  high-throughput training.

## Quality Checklist

Before trusting a trained output:

- Save the exact training command.
- Save `llama-finetune --version`.
- Save the base model checksum.
- Save the training data checksum.
- Keep a copy of the unquantized trained GGUF.
- Quantize into a separate file; do not overwrite the trained GGUF.
- Test the trained model through `llama-cli` before serving it through agents.
- Compare outputs against a small holdout prompt set.

## Troubleshooting

### `train failed: installed llama-finetune does not advertise --lora-out`

You are using official upstream-style `llama-finetune`. Use full-model output:

```sh
lmml train --model-base ./base.gguf --train-data ./data/train.txt --output ./outputs/tuned.gguf -- --epochs 1
```

### Training data appears ignored

Check that the file is plain text and that examples use the right chat template.
For chat models, raw `{"messages": ...}` JSONL is not enough unless rendered into
role-marked text.

### Output quality is worse after training

Common causes:

- Base model was quantized.
- Dataset format does not match the model's chat template.
- Dataset is too small, repetitive, or contradictory.
- Too many epochs caused overfitting.
- Training context was too short for the examples.

### Fine-tuned model is too large for serving

Quantize after training:

```sh
llama-quantize tuned.gguf tuned-Q8_0.gguf Q8_0
```

Then select the quantized model in lmml.

## Recommended Defaults

Start here unless you have measured better settings:

```sh
lmml train \
  --model-base ./models/base-BF16.gguf \
  --train-data ./data/train.txt \
  --output ./outputs/tuned.gguf \
  -- --epochs 1 --ctx-size 512 --batch-size 1 --ubatch-size 1 --n-gpu-layers 0
```

After the smoke run succeeds, increase one variable at a time.
