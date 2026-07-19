#!/usr/bin/env bash
set -euo pipefail

GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m'
BOLD='\033[1m'

MODE=${LMML_INSTALL_MODE:-binary}
FIX_DEPS=${LMML_FIX_DEPS:-0}
GPU_MODE=${LMML_GPU_MODE:-required}
HARD_FAILURES=0
GPU_FAILURES=0
APT_PACKAGES=()

section() {
  printf '\n%b%s%b\n' "$BOLD" "$1" "$NC"
}

ok() {
  printf '  %b✓%b %s\n' "$GREEN" "$NC" "$1"
}

warn() {
  printf '  %b⚠%b %s\n' "$YELLOW" "$NC" "$1"
}

fail() {
  printf '  %b✗%b %s\n' "$RED" "$NC" "$1"
  HARD_FAILURES=$((HARD_FAILURES + 1))
}

gpu_fail() {
  printf '  %b✗%b %s\n' "$RED" "$NC" "$1"
  GPU_FAILURES=$((GPU_FAILURES + 1))
}

need_apt() {
  case "$(uname -s)" in
    Linux)
      if command -v apt-get >/dev/null 2>&1; then
        APT_PACKAGES+=("$1")
      fi
      ;;
  esac
}

first_version() {
  sed -n 's/[^0-9]*\([0-9][0-9.]*\).*/\1/p' | head -n 1
}

major_version() {
  printf '%s\n' "$1" | sed -n 's/[^0-9]*\([0-9][0-9]*\).*/\1/p' | head -n 1
}

version_ge() {
  local current=$1
  local minimum=$2
  awk -v c="$current" -v m="$minimum" '
    function splitv(v, out) {
      n = split(v, parts, ".")
      for (i = 1; i <= 3; i++) out[i] = (i <= n ? parts[i] + 0 : 0)
    }
    BEGIN {
      splitv(c, cv); splitv(m, mv)
      for (i = 1; i <= 3; i++) {
        if (cv[i] > mv[i]) exit 0
        if (cv[i] < mv[i]) exit 1
      }
      exit 0
    }'
}

first_line() {
  sed -n '1p'
}

check_version_command() {
  local name=$1
  local command=$2
  if command -v "$command" >/dev/null 2>&1; then
    if output=$("$command" --version 2>&1); then
      ok "$name $(printf '%s\n' "$output" | first_version)"
    else
      fail "$name exists but is not usable: $(printf '%s\n' "$output" | first_line)"
    fi
  else
    fail "$name not found"
  fi
}

case "$MODE" in
  binary|source) ;;
  *)
    printf '%b✗ Unsupported LMML_INSTALL_MODE=%s%b\n' "$RED" "$MODE" "$NC" >&2
    printf '  Use LMML_INSTALL_MODE=binary or LMML_INSTALL_MODE=source.\n' >&2
    exit 2
    ;;
esac

case "$GPU_MODE" in
  required|cpu-only|vulkan) ;;
  *)
    printf '%b✗ Unsupported LMML_GPU_MODE=%s%b\n' "$RED" "$GPU_MODE" "$NC" >&2
    printf '  Use LMML_GPU_MODE=required, LMML_GPU_MODE=vulkan, or LMML_GPU_MODE=cpu-only.\n' >&2
    exit 2
    ;;
esac

section "LMML PREFLIGHT"
printf 'Mode: %s\n' "$MODE"
printf 'GPU mode: %s\n' "$GPU_MODE"

section "SYSTEM ARCHITECTURE & OS"
OS=$(uname -s)
ARCH=$(uname -m)
printf 'OS: %s\nArchitecture: %s\n' "$OS" "$ARCH"
if [[ "$OS" == "Linux" && ("$ARCH" == "x86_64" || "$ARCH" == "aarch64" || "$ARCH" == "arm64") ]] ||
   [[ "$OS" == "Darwin" && ("$ARCH" == "x86_64" || "$ARCH" == "arm64" || "$ARCH" == "aarch64") ]]; then
  ok "supported platform"
else
  fail "unsupported platform: $OS $ARCH"
fi

section "DISK SPACE"
AVAILABLE_GB=$(df -Pk . | awk 'NR == 2 { printf "%d", $4 / 1024 / 1024 }')
printf 'Available storage: %s GB\n' "$AVAILABLE_GB"
if (( AVAILABLE_GB >= 4 )); then
  ok "at least 4 GB free"
else
  fail "less than 4 GB free"
fi

section "DOWNLOAD TOOLS"
if command -v curl >/dev/null 2>&1 || command -v wget >/dev/null 2>&1; then
  command -v curl >/dev/null 2>&1 && ok "curl found: $(command -v curl)"
  command -v wget >/dev/null 2>&1 && ok "wget found: $(command -v wget)"
else
  fail "curl or wget is required"
  need_apt curl
fi

section "BUILD PREREQUISITES"
if command -v g++ >/dev/null 2>&1; then
  ok "g++ found: $(g++ -dumpfullversion -dumpversion)"
elif command -v clang++ >/dev/null 2>&1; then
  ok "clang++ found: $(clang++ --version | head -n 1)"
else
  fail "C++17 compiler not found"
  need_apt build-essential
fi

if command -v cmake >/dev/null 2>&1; then
  CMAKE_VERSION=$(cmake --version | first_version)
  if version_ge "$CMAKE_VERSION" "3.21"; then
    ok "cmake $CMAKE_VERSION"
  else
    fail "cmake $CMAKE_VERSION found; 3.21 required"
    need_apt cmake
  fi
else
  fail "cmake not found"
  need_apt cmake
fi

if command -v git >/dev/null 2>&1; then
  GIT_VERSION=$(git --version | first_version)
  if version_ge "$GIT_VERSION" "2.28"; then
    ok "git $GIT_VERSION"
  else
    fail "git $GIT_VERSION found; 2.28 required"
    need_apt git
  fi
else
  fail "git not found"
  need_apt git
fi

if command -v sccache >/dev/null 2>&1; then
  SCCACHE_VERSION=$(sccache --version | first_version)
  ok "sccache ${SCCACHE_VERSION:-found} — llama.cpp rebuilds will be faster"
else
  warn "sccache not found — recommended for faster llama.cpp rebuilds"
  need_apt sccache
fi

section "RUST TOOLCHAIN"
if [[ "$MODE" == "source" ]]; then
  check_version_command rustc rustc
  check_version_command cargo cargo
  if command -v rustup >/dev/null 2>&1; then
    if RUSTUP_TOOLCHAIN=$(rustup show active-toolchain 2>&1 | head -n 1); then
      ok "rustup $RUSTUP_TOOLCHAIN"
    else
      fail "rustup exists but no active toolchain is configured: $RUSTUP_TOOLCHAIN"
    fi
  else
    fail "rustup not found"
  fi
else
  if command -v rustc >/dev/null 2>&1 && command -v cargo >/dev/null 2>&1; then
    ok "Rust present, not required for binary install"
  else
    warn "Rust not found; not required for binary install"
  fi
fi

section "GPU ACCELERATION"
if [[ "$GPU_MODE" == "cpu-only" ]]; then
  warn "CPU-only mode selected; GPU acceleration checks are informational."
elif [[ "$GPU_MODE" == "vulkan" ]]; then
  printf 'Vulkan GPU acceleration selected for lmml preflight.\n'
else
  printf 'GPU acceleration is primary and first-class for lmml preflight.\n'
fi

if command -v nvidia-smi >/dev/null 2>&1; then
  if NVIDIA_SMI_OUTPUT=$(nvidia-smi --query-gpu=name,memory.total,compute_cap --format=csv,noheader 2>&1); then
    ok "NVIDIA driver/GPU probe succeeded"
    printf '%s\n' "$NVIDIA_SMI_OUTPUT" | sed 's/^/    /'
  else
    if [[ "$GPU_MODE" == "cpu-only" || "$GPU_MODE" == "vulkan" ]]; then
      warn "nvidia-smi failed: $NVIDIA_SMI_OUTPUT"
    else
      gpu_fail "nvidia-smi failed: $NVIDIA_SMI_OUTPUT"
    fi
  fi
else
  if [[ "$GPU_MODE" == "cpu-only" || "$GPU_MODE" == "vulkan" ]]; then
    warn "nvidia-smi not found"
  else
    gpu_fail "nvidia-smi not found"
  fi
fi

if command -v nvcc >/dev/null 2>&1; then
  NVCC_VERSION=$(nvcc --version | sed -n 's/.*release \([^,]*\).*/\1/p' | head -n 1)
  ok "nvcc ${NVCC_VERSION:-found}"
  GCC_MAJOR=""
  if command -v g++ >/dev/null 2>&1; then
    GCC_MAJOR=$(major_version "$(g++ -dumpfullversion -dumpversion)")
  fi
  NVCC_MAJOR=$(major_version "$NVCC_VERSION")
  if [[ ( "$NVCC_MAJOR" == "11" || "${NVCC_MAJOR:-0}" -ge 13 ) && "${GCC_MAJOR:-0}" -ge 13 ]]; then
    if command -v g++-11 >/dev/null 2>&1; then
      ok "g++-11 found — CUDA ${NVCC_VERSION} host compiler workaround available"
    else
      warn "CUDA ${NVCC_VERSION} with GCC ${GCC_MAJOR} can fail on CUDA/glibc math headers"
      warn "Install g++-11 so lmml can pass -DCMAKE_CUDA_HOST_COMPILER=/usr/bin/g++-11"
      need_apt g++-11
    fi
  fi
else
  if [[ "$GPU_MODE" == "cpu-only" || "$GPU_MODE" == "vulkan" ]]; then
    warn "nvcc not found"
  else
    gpu_fail "nvcc not found"
  fi
fi

if command -v vulkaninfo >/dev/null 2>&1; then
  if vulkaninfo --summary >/dev/null 2>&1; then
    ok "Vulkan runtime available"
  else
    if [[ "$GPU_MODE" == "vulkan" ]]; then
      gpu_fail "vulkaninfo found but summary probe failed"
    else
      warn "vulkaninfo found but summary probe failed"
    fi
  fi
elif command -v ldconfig >/dev/null 2>&1 && ldconfig -p 2>/dev/null | grep -q 'libvulkan'; then
  ok "libvulkan found"
else
  if [[ "$GPU_MODE" == "vulkan" ]]; then
    gpu_fail "Vulkan runtime not detected"
  else
    warn "Vulkan runtime not detected"
  fi
fi

if [[ "$OS" == "Darwin" ]]; then
  if system_profiler SPDisplaysDataType 2>/dev/null | grep -q "Metal"; then
    ok "Metal supported"
  else
    if [[ "$GPU_MODE" == "cpu-only" ]]; then
      warn "Metal support not detected"
    else
      gpu_fail "Metal support not detected"
    fi
  fi
else
  printf '  Metal: N/A on %s\n' "$OS"
fi

section "RESOLUTION"
if (( ${#APT_PACKAGES[@]} > 0 )); then
  mapfile -t UNIQUE_APT_PACKAGES < <(printf '%s\n' "${APT_PACKAGES[@]}" | awk '!seen[$0]++')
  if [[ "$FIX_DEPS" == "1" ]]; then
    printf 'Installing missing apt packages: %s\n' "${UNIQUE_APT_PACKAGES[*]}"
    sudo apt-get update
    sudo apt-get install -y "${UNIQUE_APT_PACKAGES[@]}"
  else
    warn "apt packages may fix hard or recommended prerequisites: ${UNIQUE_APT_PACKAGES[*]}"
    printf '  Re-run with LMML_FIX_DEPS=1 to install compiler/cmake/git/curl/sccache/g++-11 via apt.\n'
  fi
fi

if (( HARD_FAILURES > 0 || GPU_FAILURES > 0 )); then
  printf '\n%bPreflight failed:%b %d hard prerequisite failure(s), %d first-class GPU acceleration failure(s).\n' "$RED" "$NC" "$HARD_FAILURES" "$GPU_FAILURES"
  printf '  For Vulkan-only nodes, re-run with LMML_GPU_MODE=vulkan.\n'
  printf '  For intentional CPU-only nodes, re-run with LMML_GPU_MODE=cpu-only.\n'
  exit 1
fi

if [[ "$GPU_MODE" == "cpu-only" ]]; then
  printf '\n%bPreflight passed.%b Hard prerequisites passed for an intentional CPU-only node.\n' "$GREEN" "$NC"
elif [[ "$GPU_MODE" == "vulkan" ]]; then
  printf '\n%bPreflight passed.%b Hard prerequisites and Vulkan acceleration checks passed.\n' "$GREEN" "$NC"
else
  printf '\n%bPreflight passed.%b Hard prerequisites and first-class GPU acceleration checks passed.\n' "$GREEN" "$NC"
fi
