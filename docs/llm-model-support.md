# LMML LLM Model Support Catalog

LMML treats model-family support differently from runtime profile support.

Runtime profiles are exact and hardware-specific. The model catalog is broader:
it recognizes known local LLM variants and provides practical serving guidance
in the Models tab without claiming every size has been field-validated.

## Policy

- Add all important open variants to the catalog when their family/size is
  stable enough to recognize by GGUF filename.
- Add built-in runtime profiles only for combinations that have actionable
  launch settings and hardware expectations.
- Prefer advisory guidance over unvalidated profile sprawl.
- Use embedded GGUF chat templates unless a model-specific override is validated.

## Qwen3.5

Qwen3.5 is included as the broad multimodal local-AI family LMML has already
been validating against.

| Variant | Architecture | Context | LMML Use |
| --- | --- | ---: | --- |
| Qwen3.5-0.8B | Dense | 262k | tiny development, fine-tune experiments, CPU fallback |
| Qwen3.5-2B | Dense | 262k | small fast-agent path |
| Qwen3.5-4B | Dense | 262k | validated 11GB-16GB long-context coding target |
| Qwen3.5-9B | Dense | 262k | validated 24GB workstation coding target |
| Qwen3.5-27B | Dense | 262k | 24GB+ Q4 target; prefer 32GB+ for long context |
| Qwen3.5-35B-A3B | MoE, 3B active | 262k | MoE coding/reasoning target |
| Qwen3.5-122B-A10B | MoE, 10B active | 262k | multi-GPU or server-class target |
| Qwen3.5-397B-A17B | MoE, 17B active | 262k | datacenter-scale local inference only |

Existing LMML runtime profiles remain focused on the field-tested 4B and 9B
Q8 paths. The larger variants are catalog-aware but should get dedicated
profiles only after VRAM/KV-cache behavior is validated.

## Qwen3.6

Qwen3.6 should be added, but not as every guessed size. The official open
Qwen3.6 variants currently surfaced by Qwen are:

| Variant | Architecture | Context | LMML Use |
| --- | --- | ---: | --- |
| Qwen3.6-27B | Dense | 262k | current dense Qwen3.6 local flagship |
| Qwen3.6-35B-A3B | MoE, 3B active | 262k | current Qwen3.6 agentic-coding MoE |

LMML should not invent Qwen3.6 0.8B, 2B, 4B, 9B, 12B, 26B, or 31B entries until
those specific Qwen3.6 variants are officially released or a specific GGUF repo
is intentionally supported as a community build.

## Qwen Execution Policy

LMML applies Qwen-specific launch safeguards in the final server config assembly
for Qwen3.5 and Qwen3.6 models:

- Add `--reasoning-format none` so llama.cpp returns the raw stream, preserving
  `<think>` and `</think>` tags for downstream parsing.
- When context is larger than 32768 tokens, add `-ctk q8_0 -ctv q8_0` unless the
  selected profile already set key/value cache types.
- Preserve explicit profile choices. For example, `--kv-unified` fanout profiles
  that intentionally use `q4_0` KV are not rewritten to `q8_0`.
- Keep `chat_template` empty by default so GGUF embedded templates are used.
- Support explicit template overrides through LMML settings when a specific
  Qwen Jinja template has been locally validated.

LMML does not currently bundle third-party Qwen template files. Community fixed
templates may be useful, but bundling them requires a source, license, and
regression decision. The safe default is to expose template override support
without silently replacing model-provided templates.

## Gemma 4

Gemma 4 has five official sizes. LMML tracks all five in the catalog.

| Variant | Architecture | Context | LMML Use |
| --- | --- | ---: | --- |
| Gemma 4 E2B | Effective dense, 5.1B loaded | 128k | mobile, browser, and edge hardware |
| Gemma 4 E4B | Effective dense, 8B loaded | 128k | small local agents on 8GB-12GB systems |
| Gemma 4 12B | Dense unified multimodal | 256k | 16GB QAT/MTP target |
| Gemma 4 26B-A4B | MoE, 4B active | 256k | efficient larger Gemma route |
| Gemma 4 31B | Dense | 256k | heaviest dense Gemma 4 local developer target |

Implementation notes baked into LMML:

- E2B and E4B are effective-size models using Per-Layer Embeddings for edge
  hardware.
- 12B is an encoder-free unified multimodal model that maps vision and audio
  through direct linear projections.
- 26B-A4B is a mixture-of-experts model activating about 4B parameters per token.
- 31B is the largest dense Gemma 4 variant for server-grade local workstations.
- Gemma 4 natively supports `system`, `user`, and `assistant` roles.
- Thinking mode is toggled with `<|think|>` in the system prompt.
- Prefer Google-provided QAT `q4_0` GGUF checkpoints where available for
  near-lossless 4-bit execution.
- All Gemma 4 sizes include MTP draft support; a matching draft GGUF is required
  for llama.cpp speculative decoding.

## Hermes 4

Hermes support is catalog-level first. LMML recognizes the current Nous Research
Hermes 4 family by GGUF filename and surfaces ChatML/reasoning guidance, but it
does not add hardware runtime profiles until specific GGUF builds are validated.

| Variant | Architecture | Context | LMML Use |
| --- | --- | ---: | --- |
| Hermes 4 14B | Dense, Qwen 3 based | 40,960 | local coding/reasoning target; 16GB Q4 possible, 24GB preferred |
| Hermes 4.3 36B | Dense, Seed 36B based | 524k | large local/server target; 32GB+ Q4 or multi-GPU recommended |
| Hermes 4 70B | Dense, Llama 3.1 based | 131k | server or multi-GPU target |
| Hermes 4 405B FP8 | Dense, Llama 3.1 based | 131k | datacenter-scale target |

Implementation notes baked into LMML:

- Hermes 4 14B uses ChatML formatting and can run in hybrid reasoning mode with
  `<think>...</think>` sections.
- Hermes tool-call tags are model-specific. Validate downstream parsing before
  enabling tool execution in an agent harness.
- Hermes 4 14B recommended sampling starts at `temperature=0.6`, `top_p=0.95`,
  and `top_k=20`. Larger Hermes model cards/configs should be checked per GGUF
  before hard-coding runtime defaults.
- Because Hermes variants are based on different upstream architectures, LMML
  should not reuse Qwen/Gemma runtime flags blindly. Keep profiles explicit.

## Source Anchors

- Qwen3.6 repository:
  <https://github.com/QwenLM/Qwen3.6>
- Qwen3.5 GGUF collection:
  <https://huggingface.co/collections/unsloth/qwen35>
- Gemma 4 model overview:
  <https://ai.google.dev/gemma/docs/core>
- Gemma 4 model card:
  <https://ai.google.dev/gemma/docs/core/model_card_4>
- Gemma 4 MTP overview:
  <https://ai.google.dev/gemma/docs/mtp/overview>
- Hermes 4 14B model card:
  <https://huggingface.co/NousResearch/Hermes-4-14B>
- Hermes 4.3 36B model card:
  <https://huggingface.co/NousResearch/Hermes-4.3-36B>
- Hermes 4 70B model card:
  <https://huggingface.co/NousResearch/Hermes-4-70B>
- Hermes 4 405B FP8 model card:
  <https://huggingface.co/NousResearch/Hermes-4-405B-FP8>
