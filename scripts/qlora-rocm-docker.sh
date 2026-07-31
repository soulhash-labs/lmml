#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
IMAGE=${LMML_QLORA_IMAGE:-rocm/pytorch:rocm7.2.4_ubuntu24.04_py3.12_pytorch_release_2.9.1}
USER_HOME=$HOME
if [[ -n "${SUDO_USER:-}" ]]; then
  detected_home=$(getent passwd "$SUDO_USER" | cut -d: -f6 || true)
  if [[ -n "$detected_home" ]]; then
    USER_HOME=$detected_home
  fi
fi
TRAINING_SOURCE_ROOT=${TRAINING_SOURCE_ROOT:-$ROOT_DIR/../training-source}
LMML_DATA_ROOT=${LMML_DATA_ROOT:-$USER_HOME/.local/share/lmml}
HF_CACHE=${HF_CACHE:-$USER_HOME/.cache/huggingface}
PIP_CACHE=${PIP_CACHE:-$USER_HOME/.cache/pip}
QLORA_DEPS_ROOT=${LMML_QLORA_DEPS_ROOT:-$LMML_DATA_ROOT/qlora-python}
SHM_SIZE=${LMML_QLORA_SHM_SIZE:-16G}
MODE=${1:-shell}

usage() {
  cat <<'USAGE'
Usage: scripts/qlora-rocm-docker.sh [shell|doctor|matrix|prepare|smoke|train|convert|smoke-convert|reset-deps]

Environment overrides:
  LMML_QLORA_IMAGE          ROCm PyTorch image tag
  TRAINING_SOURCE_ROOT      Host path containing safe_tensors/ and data/train.jsonl
  LMML_DATA_ROOT            Host ~/.local/share/lmml path
  LMML_QLORA_DEPS_ROOT      Host path for persistent Python adapter deps
  LMML_QLORA_INSTALL_DEPS   Set 0 to skip pip dependency installation
  MODEL_ID                  Container model path
  DATA_PATH                 Container training JSONL path
  OUT_DIR                   Container adapter output dir
  ADAPTER_GGUF_OUT          Container GGUF adapter output path
  SEQ_LEN LORA_R GRAD_ACCUM NUM_EPOCHS MAX_STEPS MAX_SAMPLES LR COMPUTE_DTYPE
  OPTIM EMPTY_CACHE_STEPS LOG_MEMORY_STEPS SAVE_STRATEGY SAVE_STEPS SAVE_TOTAL_LIMIT
  TRACE_BATCH_ROWS SUSPECT_ROWS ONLY_SOURCE_LINES PAD_TO_MULTIPLE_OF SHUFFLE_DATA
  TRACE_BACKWARD_MODULES BACKWARD_TRACE_MARKERS TRACE_BACKWARD_PREFIX LORA_EXCLUDE_MODULES
  LORA_AUTOCAST_ADAPTER_DTYPE ROCM_BLAS_BACKEND ROCBLAS_USE_HIPBLASLT
  DATALOADER_NUM_WORKERS DATALOADER_PIN_MEMORY DATALOADER_PERSISTENT_WORKERS
  ATTN_IMPLEMENTATION FORCE_MATH_SDP
  HIP_LAUNCH_BLOCKING AMD_SERIALIZE_KERNEL AMD_SERIALIZE_COPY TORCH_SHOW_CPP_STACKTRACES
  PYTHONFAULTHANDLER PYTORCH_NO_HIP_MEMORY_CACHING TORCH_DISABLE_ADDR2LINE
USAGE
}

if [[ "$MODE" == "-h" || "$MODE" == "--help" ]]; then
  usage
  exit 0
fi

case "$MODE" in
  shell|doctor|matrix|prepare|smoke|train|convert|smoke-convert|reset-deps) ;;
  *)
    usage >&2
    exit 2
    ;;
esac

if ! docker info >/dev/null 2>&1; then
  cat >&2 <<'EOF'
Docker is installed, but this user cannot access the Docker daemon.

Fix by adding the user to the docker group and starting a new login session:

  sudo usermod -aG docker "$USER"

Or run this script through an approved root-owned workflow. The QLoRA container
needs /dev/kfd and /dev/dri access for ROCm.
EOF
  exit 1
fi

if [[ ! -d "$TRAINING_SOURCE_ROOT/safe_tensors" ]]; then
  echo "missing external safetensors directory: $TRAINING_SOURCE_ROOT/safe_tensors" >&2
  exit 1
fi
if [[ ! -f "$TRAINING_SOURCE_ROOT/data/train.jsonl" ]]; then
  echo "missing external chat JSONL dataset under: $TRAINING_SOURCE_ROOT/data" >&2
  exit 1
fi
if [[ ! -d "$LMML_DATA_ROOT/llama.cpp" ]]; then
  echo "missing lmml llama.cpp checkout: $LMML_DATA_ROOT/llama.cpp" >&2
  exit 1
fi

patch_lora_converter_tensor_shape_methods() {
  local converter="$LMML_DATA_ROOT/llama.cpp/convert_lora_to_gguf.py"
  if [[ ! -f "$converter" ]]; then
    return
  fi
  if grep -q "def dim(self) -> int:" "$converter"; then
    return
  fi
  python3 - "$converter" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text()
old = """    def size(self, dim=None):
        assert dim is None
        return self.shape

    def contiguous"""
new = """    def size(self, dim=None):
        if dim is None:
            return self.shape
        return self.shape[dim]

    def dim(self) -> int:
        return len(self.shape)

    @property
    def ndim(self) -> int:
        return len(self.shape)

    def contiguous"""
if old not in text:
    raise SystemExit("unsupported convert_lora_to_gguf.py LoraTorchTensor layout")
path.write_text(text.replace(old, new, 1))
PY
  echo "patched llama.cpp LoRA converter tensor shape methods: $converter"
}

patch_qwen_lora_out_proj_reorder() {
  local qwen_converter="$LMML_DATA_ROOT/llama.cpp/conversion/qwen.py"
  if [[ ! -f "$qwen_converter" ]]; then
    return
  fi
  if grep -q 'hasattr(data_torch, "get_lora_A_B")' "$qwen_converter"; then
    return
  fi
  python3 - "$qwen_converter" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text()
old = """            elif ".out_proj." in name:
                # Out projection weight: reorder columns (input dimension)
                data_torch = self._reorder_v_heads(data_torch, 1, num_k_heads, num_v_per_k, head_v_dim)

        yield from super().modify_tensors(data_torch, name, bid)
"""
new = """            elif ".out_proj." in name:
                # Out projection weight: reorder columns (input dimension)
                if hasattr(data_torch, "get_lora_A_B"):
                    lora_a, lora_b = data_torch.get_lora_A_B()
                    col_perm = self._reorder_v_heads(
                        torch.arange(num_v_heads * head_v_dim, dtype=torch.long).unsqueeze(0),
                        1, num_k_heads, num_v_per_k, head_v_dim,
                    ).squeeze(0)
                    lora_a = lora_a.index_select(-1, col_perm.to(device=lora_a.device))
                    data_torch = type(data_torch)(lora_a, lora_b)
                else:
                    data_torch = self._reorder_v_heads(data_torch, 1, num_k_heads, num_v_per_k, head_v_dim)

        yield from super().modify_tensors(data_torch, name, bid)
"""
if old not in text:
    raise SystemExit("unsupported qwen.py linear_attn out_proj reorder layout")
path.write_text(text.replace(old, new, 1))
PY
  echo "patched llama.cpp Qwen LoRA out_proj reorder: $qwen_converter"
}

container_path_to_host() {
  local path=$1
  case "$path" in
    /workspace/lmml)
      printf '%s\n' "$ROOT_DIR"
      ;;
    /workspace/lmml/*)
      printf '%s/%s\n' "$ROOT_DIR" "${path#/workspace/lmml/}"
      ;;
    /workspace/training-source)
      printf '%s\n' "$TRAINING_SOURCE_ROOT"
      ;;
    /workspace/training-source/*)
      printf '%s/%s\n' "$TRAINING_SOURCE_ROOT" "${path#/workspace/training-source/}"
      ;;
    /host-lmml)
      printf '%s\n' "$LMML_DATA_ROOT"
      ;;
    /host-lmml/*)
      printf '%s/%s\n' "$LMML_DATA_ROOT" "${path#/host-lmml/}"
      ;;
    *)
      printf '%s\n' "$path"
      ;;
  esac
}

repair_qwen35_lora_gguf() {
  local host_out_dir host_lora_gguf host_base_config
  host_out_dir=$(container_path_to_host "$OUT_DIR")
  host_lora_gguf=$(container_path_to_host "$ADAPTER_GGUF_OUT")
  host_base_config="$TRAINING_SOURCE_ROOT/safe_tensors/config.json"
  PYTHONPATH="$LMML_DATA_ROOT/llama.cpp/gguf-py:${PYTHONPATH:-}" \
    python3 "$ROOT_DIR/scripts/repair_qwen35_lora_gguf.py" \
      --adapter-dir "$host_out_dir" \
      --lora-gguf "$host_lora_gguf" \
      --base-config "$host_base_config"
}

case "$MODE" in
  convert|smoke-convert)
    patch_lora_converter_tensor_shape_methods
    patch_qwen_lora_out_proj_reorder
    ;;
esac

mkdir -p "$ROOT_DIR/outputs/qlora" "$HF_CACHE" "$PIP_CACHE" "$QLORA_DEPS_ROOT"

if [[ "$MODE" == "reset-deps" ]]; then
  if find "$QLORA_DEPS_ROOT" -mindepth 1 -maxdepth 1 -print -quit | grep -q .; then
    backup="${QLORA_DEPS_ROOT}.bad.$(date +%Y%m%d%H%M%S)"
    mv "$QLORA_DEPS_ROOT" "$backup"
    echo "moved existing QLoRA dependency target to: $backup"
  else
    echo "QLoRA dependency target is already empty: $QLORA_DEPS_ROOT"
  fi
  mkdir -p "$QLORA_DEPS_ROOT"
  exit 0
fi

MODEL_ID=${MODEL_ID:-/workspace/training-source/safe_tensors}
DATA_PATH=${DATA_PATH:-/workspace/training-source/data/train.jsonl}
if [[ -z "${OUT_DIR:-}" ]]; then
  case "$MODE" in
    smoke|smoke-convert)
      OUT_DIR=/workspace/lmml/outputs/qlora/qwen35-27b-r9700-qlora-smoke
      ;;
    *)
      OUT_DIR=/workspace/lmml/outputs/qlora/qwen35-27b-r9700-qlora
      ;;
  esac
fi
if [[ -z "${ADAPTER_GGUF_OUT:-}" ]]; then
  case "$MODE" in
    smoke|smoke-convert)
      ADAPTER_GGUF_OUT=/workspace/lmml/outputs/qlora/qwen35-27b-r9700-qlora-smoke.gguf
      ;;
    *)
      ADAPTER_GGUF_OUT=/workspace/lmml/outputs/qlora/qwen35-27b-r9700-qlora.gguf
      ;;
  esac
fi
LMML_QLORA_INSTALL_DEPS=${LMML_QLORA_INSTALL_DEPS:-1}
LORA_AUTOCAST_ADAPTER_DTYPE=${LORA_AUTOCAST_ADAPTER_DTYPE:-1}
ROCM_BLAS_BACKEND=${ROCM_BLAS_BACKEND:-rocblas}
ROCBLAS_USE_HIPBLASLT=${ROCBLAS_USE_HIPBLASLT:-0}

GROUP_IDS=()
GROUP_ARGS=()

add_group_id() {
  local gid=$1
  local existing
  if [[ -z "$gid" ]]; then
    return
  fi
  for existing in "${GROUP_IDS[@]}"; do
    if [[ "$existing" == "$gid" ]]; then
      return
    fi
  done
  GROUP_IDS+=("$gid")
  GROUP_ARGS+=(--group-add "$gid")
}

add_group_name() {
  local name=$1
  local entry
  entry=$(getent group "$name" || true)
  if [[ -n "$entry" ]]; then
    add_group_id "$(printf '%s\n' "$entry" | cut -d: -f3)"
  fi
}

add_device_group() {
  local path=$1
  if [[ -e "$path" ]]; then
    add_group_id "$(stat -c '%g' "$path")"
  fi
}

add_group_name video
add_group_name render
add_device_group /dev/kfd
for dri_device in /dev/dri/*; do
  add_device_group "$dri_device"
done

DEBUG_ENV_ARGS=(
  -e "HIP_LAUNCH_BLOCKING=${HIP_LAUNCH_BLOCKING:-0}"
  -e "AMD_SERIALIZE_KERNEL=${AMD_SERIALIZE_KERNEL:-0}"
  -e "AMD_SERIALIZE_COPY=${AMD_SERIALIZE_COPY:-0}"
  -e "TORCH_SHOW_CPP_STACKTRACES=${TORCH_SHOW_CPP_STACKTRACES:-0}"
  -e "PYTHONFAULTHANDLER=${PYTHONFAULTHANDLER:-0}"
  -e "PYTORCH_NO_HIP_MEMORY_CACHING=${PYTORCH_NO_HIP_MEMORY_CACHING:-0}"
  -e "TORCH_DISABLE_ADDR2LINE=${TORCH_DISABLE_ADDR2LINE:-0}"
  -e "ROCBLAS_USE_HIPBLASLT=${ROCBLAS_USE_HIPBLASLT:-}"
)

container_cmd='set -euo pipefail
export LMML_QLORA_DEPS_DIR="${LMML_QLORA_DEPS_DIR:-/host-lmml/qlora-python}"
mkdir -p "${LMML_QLORA_DEPS_DIR}"

contaminated_modules() {
  python3 - <<'"'"'PY'"'"'
from pathlib import Path
import os
root = Path(os.environ["LMML_QLORA_DEPS_DIR"])
names = ["torch", "torchgen", "functorch", "triton", "nvidia"]
bad = []
children = root.iterdir() if root.exists() else []
for child in children:
    name = child.name
    if name in names or any(name.startswith(prefix) for prefix in ["torch-", "triton-", "nvidia_"]):
        bad.append(name)
print(" ".join(sorted(bad)))
PY
}

contaminated="$(contaminated_modules)"
if [[ -n "${contaminated}" ]]; then
  echo "contaminated QLoRA dependency target contains ROCm-shadowing packages: ${contaminated}" >&2
  echo "run: scripts/qlora-rocm-docker.sh reset-deps" >&2
  exit 1
fi

export PYTHONPATH="${LMML_QLORA_DEPS_DIR}:${PYTHONPATH:-}"

broken_modules() {
  python3 - <<'"'"'PY'"'"'
import importlib
modules = ["torch", "transformers", "peft", "bitsandbytes", "safetensors"]
broken = []
for name in modules:
    try:
        importlib.import_module(name)
    except Exception as error:
        broken.append(f"{name}:{type(error).__name__}")
print(" ".join(broken))
PY
}

broken="$(broken_modules)"
if [[ "${LMML_QLORA_INSTALL_DEPS}" != "0" && -n "${broken}" ]]; then
  python3 -m pip install --upgrade pip
  python3 -m pip install --upgrade --no-deps --target "${LMML_QLORA_DEPS_DIR}" -r /workspace/lmml/scripts/qlora-rocm-requirements.txt
  broken="$(broken_modules)"
fi
if [[ -n "${broken}" ]]; then
  echo "broken QLoRA Python environment: ${broken}" >&2
  echo "run reset-deps, then rerun without LMML_QLORA_INSTALL_DEPS=0" >&2
  echo "dependency target: ${LMML_QLORA_DEPS_DIR}" >&2
  exit 1
fi
case "${LMML_QLORA_MODE}" in
  shell)
    exec bash
    ;;
  doctor)
    python3 - <<'"'"'PY'"'"'
import importlib
for name in ["torch", "transformers", "peft", "bitsandbytes", "safetensors"]:
    module = importlib.import_module(name)
    print(f"{name}={getattr(module, '"'"'__version__'"'"', '"'"'unknown'"'"')}")
import torch
print(f"torch.version.hip={getattr(torch.version, '"'"'hip'"'"', None)}")
print(f"torch.cuda.is_available={torch.cuda.is_available()}")
if torch.cuda.is_available():
    print(f"device={torch.cuda.get_device_name(0)}")
PY
    ;;
  matrix)
    python3 - <<'"'"'PY'"'"'
from importlib.metadata import PackageNotFoundError, version
import platform
import torch

packages = [
    "bitsandbytes",
    "transformers",
    "accelerate",
    "peft",
    "trl",
    "datasets",
]

print("python:", platform.python_version())
print("torch:", torch.__version__)
print("torch.version.hip:", getattr(torch.version, "hip", None))
print("cuda-api-visible:", torch.cuda.is_available())
if torch.cuda.is_available():
    props = torch.cuda.get_device_properties(0)
    print("device:", torch.cuda.get_device_name(0))
    print("device_properties:", props)
    print("gcn_arch:", getattr(props, "gcnArchName", "unknown"))
    allocator = (
        torch.cuda.get_allocator_backend()
        if hasattr(torch.cuda, "get_allocator_backend")
        else "unknown"
    )
    print("allocator:", allocator)
for package in packages:
    try:
        print(f"{package}:", version(package))
    except PackageNotFoundError:
        print(f"{package}: not installed")
PY
    python3 -m bitsandbytes || true
    ;;
  prepare)
    PREPARE_ONLY=1 python3 /workspace/lmml/scripts/train_qwen35_27b_r9700_qlora.py
    ;;
  smoke)
    MAX_STEPS="${MAX_STEPS:-20}" MAX_SAMPLES="${MAX_SAMPLES:-256}" SEQ_LEN="${SEQ_LEN:-512}" LORA_R="${LORA_R:-8}" GRAD_ACCUM="${GRAD_ACCUM:-8}" OPTIM="${OPTIM:-adamw_torch}" EMPTY_CACHE_STEPS="${EMPTY_CACHE_STEPS:-0}" LOG_MEMORY_STEPS="${LOG_MEMORY_STEPS:-1}" SAVE_STRATEGY="${SAVE_STRATEGY:-steps}" SAVE_STEPS="${SAVE_STEPS:-5}" SAVE_TOTAL_LIMIT="${SAVE_TOTAL_LIMIT:-2}" TRACE_BATCH_ROWS="${TRACE_BATCH_ROWS:-1}" SUSPECT_ROWS="${SUSPECT_ROWS:-42,178}" PAD_TO_MULTIPLE_OF="${PAD_TO_MULTIPLE_OF:-0}" SHUFFLE_DATA="${SHUFFLE_DATA:-1}" TRACE_BACKWARD_MODULES="${TRACE_BACKWARD_MODULES:-0}" BACKWARD_TRACE_MARKERS="${BACKWARD_TRACE_MARKERS:-Linear4bit,GatedDeltaNet,DeltaNet}" TRACE_BACKWARD_PREFIX="${TRACE_BACKWARD_PREFIX:-}" LORA_EXCLUDE_MODULES="${LORA_EXCLUDE_MODULES:-}" LORA_AUTOCAST_ADAPTER_DTYPE="${LORA_AUTOCAST_ADAPTER_DTYPE:-1}" ROCM_BLAS_BACKEND="${ROCM_BLAS_BACKEND:-}" DATALOADER_NUM_WORKERS="${DATALOADER_NUM_WORKERS:-0}" DATALOADER_PIN_MEMORY="${DATALOADER_PIN_MEMORY:-0}" DATALOADER_PERSISTENT_WORKERS="${DATALOADER_PERSISTENT_WORKERS:-0}" ATTN_IMPLEMENTATION="${ATTN_IMPLEMENTATION:-eager}" FORCE_MATH_SDP="${FORCE_MATH_SDP:-1}" python3 /workspace/lmml/scripts/train_qwen35_27b_r9700_qlora.py
    ;;
  train)
    python3 /workspace/lmml/scripts/train_qwen35_27b_r9700_qlora.py
    ;;
  convert)
    cd /workspace/llama.cpp
    PYTHONPATH="/workspace/llama.cpp:/workspace/llama.cpp/gguf-py:${PYTHONPATH:-}" python3 /workspace/llama.cpp/convert_lora_to_gguf.py --trust-remote-code --base "${MODEL_ID}" --outtype auto --outfile "${ADAPTER_GGUF_OUT}" "${OUT_DIR}"
    ;;
  smoke-convert)
    MAX_STEPS="${MAX_STEPS:-20}" MAX_SAMPLES="${MAX_SAMPLES:-256}" SEQ_LEN="${SEQ_LEN:-512}" LORA_R="${LORA_R:-8}" GRAD_ACCUM="${GRAD_ACCUM:-8}" OPTIM="${OPTIM:-adamw_torch}" EMPTY_CACHE_STEPS="${EMPTY_CACHE_STEPS:-0}" LOG_MEMORY_STEPS="${LOG_MEMORY_STEPS:-1}" SAVE_STRATEGY="${SAVE_STRATEGY:-steps}" SAVE_STEPS="${SAVE_STEPS:-5}" SAVE_TOTAL_LIMIT="${SAVE_TOTAL_LIMIT:-2}" TRACE_BATCH_ROWS="${TRACE_BATCH_ROWS:-1}" SUSPECT_ROWS="${SUSPECT_ROWS:-42,178}" PAD_TO_MULTIPLE_OF="${PAD_TO_MULTIPLE_OF:-0}" SHUFFLE_DATA="${SHUFFLE_DATA:-1}" TRACE_BACKWARD_MODULES="${TRACE_BACKWARD_MODULES:-0}" BACKWARD_TRACE_MARKERS="${BACKWARD_TRACE_MARKERS:-Linear4bit,GatedDeltaNet,DeltaNet}" TRACE_BACKWARD_PREFIX="${TRACE_BACKWARD_PREFIX:-}" LORA_EXCLUDE_MODULES="${LORA_EXCLUDE_MODULES:-}" LORA_AUTOCAST_ADAPTER_DTYPE="${LORA_AUTOCAST_ADAPTER_DTYPE:-1}" ROCM_BLAS_BACKEND="${ROCM_BLAS_BACKEND:-}" DATALOADER_NUM_WORKERS="${DATALOADER_NUM_WORKERS:-0}" DATALOADER_PIN_MEMORY="${DATALOADER_PIN_MEMORY:-0}" DATALOADER_PERSISTENT_WORKERS="${DATALOADER_PERSISTENT_WORKERS:-0}" ATTN_IMPLEMENTATION="${ATTN_IMPLEMENTATION:-eager}" FORCE_MATH_SDP="${FORCE_MATH_SDP:-1}" python3 /workspace/lmml/scripts/train_qwen35_27b_r9700_qlora.py
    cd /workspace/llama.cpp
    PYTHONPATH="/workspace/llama.cpp:/workspace/llama.cpp/gguf-py:${PYTHONPATH:-}" python3 /workspace/llama.cpp/convert_lora_to_gguf.py --trust-remote-code --base "${MODEL_ID}" --outtype auto --outfile "${ADAPTER_GGUF_OUT}" "${OUT_DIR}"
    ;;
esac'

docker run --rm -it \
  --cap-add=SYS_PTRACE \
  --security-opt seccomp=unconfined \
  --device=/dev/kfd \
  --device=/dev/dri \
  "${GROUP_ARGS[@]}" \
  --ipc=host \
  --shm-size "$SHM_SIZE" \
  -v "$ROOT_DIR:/workspace/lmml" \
  -v "$TRAINING_SOURCE_ROOT:/workspace/training-source:ro" \
  -v "$LMML_DATA_ROOT:/host-lmml" \
  -v "$LMML_DATA_ROOT/llama.cpp:/workspace/llama.cpp:ro" \
  -v "$HF_CACHE:/root/.cache/huggingface" \
  -v "$PIP_CACHE:/root/.cache/pip" \
  -w /workspace/lmml \
  -e "LMML_QLORA_MODE=$MODE" \
  -e "LMML_QLORA_INSTALL_DEPS=$LMML_QLORA_INSTALL_DEPS" \
  -e "LMML_QLORA_DEPS_DIR=/host-lmml/qlora-python" \
  -e "MODEL_ID=$MODEL_ID" \
  -e "DATA_PATH=$DATA_PATH" \
  -e "OUT_DIR=$OUT_DIR" \
  -e "ADAPTER_GGUF_OUT=$ADAPTER_GGUF_OUT" \
  -e "SEQ_LEN=${SEQ_LEN:-}" \
  -e "LORA_R=${LORA_R:-}" \
  -e "LORA_ALPHA=${LORA_ALPHA:-}" \
  -e "LORA_DROPOUT=${LORA_DROPOUT:-}" \
  -e "GRAD_ACCUM=${GRAD_ACCUM:-}" \
  -e "NUM_EPOCHS=${NUM_EPOCHS:-}" \
  -e "MAX_STEPS=${MAX_STEPS:-}" \
  -e "MAX_SAMPLES=${MAX_SAMPLES:-}" \
  -e "LR=${LR:-}" \
  -e "COMPUTE_DTYPE=${COMPUTE_DTYPE:-auto}" \
  -e "OPTIM=${OPTIM:-}" \
  -e "EMPTY_CACHE_STEPS=${EMPTY_CACHE_STEPS:-}" \
  -e "LOG_MEMORY_STEPS=${LOG_MEMORY_STEPS:-}" \
  -e "SAVE_STRATEGY=${SAVE_STRATEGY:-}" \
  -e "SAVE_STEPS=${SAVE_STEPS:-}" \
  -e "SAVE_TOTAL_LIMIT=${SAVE_TOTAL_LIMIT:-}" \
  -e "TRACE_BATCH_ROWS=${TRACE_BATCH_ROWS:-}" \
  -e "SUSPECT_ROWS=${SUSPECT_ROWS:-}" \
  -e "ONLY_SOURCE_LINES=${ONLY_SOURCE_LINES:-}" \
  -e "PAD_TO_MULTIPLE_OF=${PAD_TO_MULTIPLE_OF:-}" \
  -e "SHUFFLE_DATA=${SHUFFLE_DATA:-}" \
  -e "TRACE_BACKWARD_MODULES=${TRACE_BACKWARD_MODULES:-}" \
  -e "BACKWARD_TRACE_MARKERS=${BACKWARD_TRACE_MARKERS:-}" \
  -e "TRACE_BACKWARD_PREFIX=${TRACE_BACKWARD_PREFIX:-}" \
  -e "LORA_EXCLUDE_MODULES=${LORA_EXCLUDE_MODULES:-}" \
  -e "LORA_AUTOCAST_ADAPTER_DTYPE=${LORA_AUTOCAST_ADAPTER_DTYPE:-}" \
  -e "ROCM_BLAS_BACKEND=${ROCM_BLAS_BACKEND:-}" \
  -e "DATALOADER_NUM_WORKERS=${DATALOADER_NUM_WORKERS:-}" \
  -e "DATALOADER_PIN_MEMORY=${DATALOADER_PIN_MEMORY:-}" \
  -e "DATALOADER_PERSISTENT_WORKERS=${DATALOADER_PERSISTENT_WORKERS:-}" \
  -e "ATTN_IMPLEMENTATION=${ATTN_IMPLEMENTATION:-eager}" \
  -e "FORCE_MATH_SDP=${FORCE_MATH_SDP:-1}" \
  -e "TARGET_MODULES=${TARGET_MODULES:-}" \
  "${DEBUG_ENV_ARGS[@]}" \
  "$IMAGE" \
  bash -lc "$container_cmd"

case "$MODE" in
  convert|smoke-convert)
    repair_qwen35_lora_gguf
    ;;
esac
