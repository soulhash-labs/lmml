#!/bin/sh
set -eu

BASE_URL=${BASE_URL:-https://github.com/YOUR_ORG/lmml/releases/latest}
VERSION=${VERSION:-}
INSTALL_MODE=${INSTALL_MODE:-binary}
LMML_PROFILE_HINT=${LMML_PROFILE_HINT:-}
LMML_CHECKSUM_VERIFY=${LMML_CHECKSUM_VERIFY:-optional}
TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/lmml-install.XXXXXX")

cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT INT TERM

fail() {
  echo "✗ $1" >&2
  if [ "${2:-}" ]; then
    echo "  $2" >&2
  fi
  exit 1
}

warn() {
  echo "⚠  $1" >&2
}

detect_target() {
  os=$(uname -s)
  arch=$(uname -m)
  case "$os:$arch" in
    Linux:x86_64) echo "x86_64-unknown-linux-gnu" ;;
    Linux:aarch64|Linux:arm64) echo "aarch64-unknown-linux-gnu" ;;
    Darwin:x86_64) echo "x86_64-apple-darwin" ;;
    Darwin:arm64|Darwin:aarch64) echo "aarch64-apple-darwin" ;;
    *)
      echo "✗ Unsupported platform: $(uname -s) $(uname -m)" >&2
      echo "  lmml supports Linux x86_64/aarch64 and macOS x86_64/arm64." >&2
      exit 1
      ;;
  esac
}

install_hint() {
  case "$(uname -s)" in
    Darwin)
      case "$1" in
        compiler) echo "xcode-select --install" ;;
        cmake) echo "brew install cmake" ;;
        git) echo "brew install git" ;;
        sccache) echo "brew install sccache" ;;
        gxx11) echo "CUDA 11.x workaround is Linux-only; upgrade CUDA or use a supported GCC toolchain" ;;
      esac
      ;;
    *)
      case "$1" in
        compiler) echo "sudo apt install build-essential" ;;
        cmake) echo "sudo apt install cmake" ;;
        git) echo "sudo apt install git" ;;
        sccache) echo "sudo apt install sccache" ;;
        gxx11) echo "sudo apt install g++-11" ;;
      esac
      ;;
  esac
}

version_ge() {
  current=$1
  minimum=$2
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

first_version() {
  sed -n 's/[^0-9]*\([0-9][0-9.]*\).*/\1/p' | head -n 1
}

major_version() {
  printf '%s\n' "$1" | sed -n 's/[^0-9]*\([0-9][0-9]*\).*/\1/p' | head -n 1
}

nvcc_release() {
  nvcc --version 2>/dev/null | sed -n 's/.*release \([^,]*\).*/\1/p' | head -n 1
}

check_prereqs() {
  missing=0
  if command -v gcc >/dev/null 2>&1; then
    echo "✓ gcc found"
  elif command -v clang >/dev/null 2>&1; then
    echo "✓ clang found"
  else
    echo "✗ gcc or clang not found" >&2
    echo "  → $(install_hint compiler)" >&2
    missing=1
  fi

  if command -v cmake >/dev/null 2>&1; then
    cmake_version=$(cmake --version | first_version)
    if version_ge "$cmake_version" "3.21"; then
      echo "✓ cmake $cmake_version"
    else
      echo "✗ cmake $cmake_version found; 3.21 required" >&2
      echo "  → $(install_hint cmake)" >&2
      missing=1
    fi
  else
    echo "✗ cmake not found" >&2
    echo "  → $(install_hint cmake)" >&2
    missing=1
  fi

  if command -v git >/dev/null 2>&1; then
    git_version=$(git --version | first_version)
    if version_ge "$git_version" "2.28"; then
      echo "✓ git $git_version"
    else
      echo "✗ git $git_version found; 2.28 required" >&2
      echo "  → $(install_hint git)" >&2
      missing=1
    fi
  else
    echo "✗ git not found" >&2
    echo "  → $(install_hint git)" >&2
    missing=1
  fi

  if command -v sccache >/dev/null 2>&1; then
    sccache_version=$(sccache --version | first_version)
    echo "✓ sccache ${sccache_version:-found} — repeat llama.cpp builds will be faster"
  else
    warn "sccache not found; repeat llama.cpp builds will be slower"
    echo "  → $(install_hint sccache)" >&2
  fi

  if command -v nvcc >/dev/null 2>&1; then
    nvcc_version=$(nvcc_release)
    gcc_major=""
    if command -v g++ >/dev/null 2>&1; then
      gcc_major=$(major_version "$(g++ -dumpfullversion -dumpversion)")
    fi
    nvcc_major=$(major_version "$nvcc_version")
    if { [ "$nvcc_major" = "11" ] || [ "${nvcc_major:-0}" -ge 13 ]; } && [ "${gcc_major:-0}" -ge 13 ]; then
      if command -v g++-11 >/dev/null 2>&1; then
        echo "✓ g++-11 found — CUDA $nvcc_version host compiler workaround available"
      else
        warn "CUDA $nvcc_version with GCC ${gcc_major} can fail on CUDA/glibc math headers"
        echo "  → $(install_hint gxx11)" >&2
        echo "  → lmml will pass -DCMAKE_CUDA_HOST_COMPILER=/usr/bin/g++-11 when available" >&2
      fi
    fi
  else
    warn "CUDA not found; will build CPU-only"
  fi
  command -v nvidia-smi >/dev/null 2>&1 || warn "GPU not detected; will build CPU-only"

  if [ "$missing" -ne 0 ]; then
    exit 1
  fi
}

download() {
  url=$1
  out=$2
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$url" -o "$out"
  elif command -v wget >/dev/null 2>&1; then
    wget -q "$url" -O "$out"
  else
    fail "curl or wget is required to download lmml"
  fi
}

try_download() {
  url=$1
  out=$2
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$url" -o "$out" 2>/dev/null
  elif command -v wget >/dev/null 2>&1; then
    wget -q "$url" -O "$out" 2>/dev/null
  else
    return 1
  fi
}

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{ print $1 }'
  else
    shasum -a 256 "$1" | awk '{ print $1 }'
  fi
}

target=$(detect_target)

case "$INSTALL_MODE" in
  binary|source) ;;
  *) fail "Unsupported INSTALL_MODE=$INSTALL_MODE" "Use INSTALL_MODE=binary or INSTALL_MODE=source." ;;
esac

case "$LMML_PROFILE_HINT" in
  ""|orion-qwen35-4b-q8|quadro-m6000-qwen35-9b-q8|qwen36-27b-q4|qwen36-35b-a3b-q4|gemma4-12b-mtp-q4km|bc250-qwen35-9b-q4km-vulkan) ;;
  *) fail "Unsupported LMML_PROFILE_HINT=$LMML_PROFILE_HINT" "Supported hints: orion-qwen35-4b-q8, quadro-m6000-qwen35-9b-q8, qwen36-27b-q4, qwen36-35b-a3b-q4, gemma4-12b-mtp-q4km, bc250-qwen35-9b-q4km-vulkan." ;;
esac

case "$LMML_CHECKSUM_VERIFY" in
  optional|required|off) ;;
  *) fail "Unsupported LMML_CHECKSUM_VERIFY=$LMML_CHECKSUM_VERIFY" "Use LMML_CHECKSUM_VERIFY=optional, required, or off." ;;
esac

if [ -z "$VERSION" ]; then
  download "$BASE_URL/latest" "$TMP_DIR/latest"
  VERSION=$(sed -n '1p' "$TMP_DIR/latest" | tr -d '[:space:]')
fi

sums_url="$BASE_URL/SHA256SUMS"
download "$sums_url" "$TMP_DIR/SHA256SUMS"

verify_signed_checksums() {
  mode=$LMML_CHECKSUM_VERIFY
  if [ "$mode" = "off" ]; then
    warn "signed SHA256SUMS verification disabled by LMML_CHECKSUM_VERIFY=off"
    return 0
  fi

  sig_url="$BASE_URL/SHA256SUMS.minisig"
  if ! try_download "$sig_url" "$TMP_DIR/SHA256SUMS.minisig"; then
    if [ "$mode" = "required" ]; then
      fail "signed checksum verification required, but SHA256SUMS.minisig was not available" "Publish SHA256SUMS.minisig or use LMML_CHECKSUM_VERIFY=optional for trusted LAN testing."
    fi
    warn "SHA256SUMS.minisig not found; using unsigned SHA256 integrity only"
    return 0
  fi

  if ! command -v minisign >/dev/null 2>&1; then
    if [ "$mode" = "required" ]; then
      fail "minisign is required to verify signed checksums" "Install minisign or use LMML_CHECKSUM_VERIFY=optional for trusted LAN testing."
    fi
    warn "minisign not found; using unsigned SHA256 integrity only"
    return 0
  fi

  if [ "${LMML_MINISIGN_PUBLIC_KEY:-}" ]; then
    if ! minisign -Vm "$TMP_DIR/SHA256SUMS" -x "$TMP_DIR/SHA256SUMS.minisig" -P "$LMML_MINISIGN_PUBLIC_KEY" >/dev/null; then
      fail "SHA256SUMS signature verification failed" "Check that LMML_MINISIGN_PUBLIC_KEY matches the release signing key."
    fi
  elif [ "${LMML_MINISIGN_PUBLIC_KEY_FILE:-}" ]; then
    if ! minisign -Vm "$TMP_DIR/SHA256SUMS" -x "$TMP_DIR/SHA256SUMS.minisig" -p "$LMML_MINISIGN_PUBLIC_KEY_FILE" >/dev/null; then
      fail "SHA256SUMS signature verification failed" "Check that LMML_MINISIGN_PUBLIC_KEY_FILE contains the release signing key."
    fi
  else
    if [ "$mode" = "required" ]; then
      fail "signed checksum verification required, but no minisign public key was configured" "Set LMML_MINISIGN_PUBLIC_KEY or LMML_MINISIGN_PUBLIC_KEY_FILE."
    fi
    warn "SHA256SUMS.minisig found, but no minisign public key configured; using unsigned SHA256 integrity only"
    return 0
  fi

  echo "✓ SHA256SUMS minisign signature verified"
}

verify_signed_checksums

verify_download() {
  file=$1
  url=$2
  expected=$(awk -v file="$file" '$2 == file { print $1 }' "$TMP_DIR/SHA256SUMS")
  if [ -z "$expected" ]; then
    fail "Checksum for $file not found in SHA256SUMS" "Try again or download manually from: $url"
  fi
  actual=$(sha256_file "$TMP_DIR/$file")
  if [ "$actual" != "$expected" ]; then
    rm -f "$TMP_DIR/$file"
    fail "Checksum verification failed for $file. The download may be corrupt." "Try again or download manually from: $url"
  fi
}

if [ "${PREFIX:-}" ]; then
  install_dir="$PREFIX/bin"
elif [ "$(id -u)" -eq 0 ]; then
  install_dir="/usr/local/bin"
else
  install_dir="$HOME/.local/bin"
fi

install_binary_and_uninstaller() {
  binary=$1
  uninstaller=$2
  node_binary=${3:-}
  router_binary=${4:-}
  mkdir -p "$install_dir"
  cp "$binary" "$install_dir/lmml"
  chmod 755 "$install_dir/lmml"
  if [ -n "$node_binary" ]; then
    cp "$node_binary" "$install_dir/lmml-node"
    chmod 755 "$install_dir/lmml-node"
  fi
  if [ -n "$router_binary" ]; then
    cp "$router_binary" "$install_dir/lmml-router"
    chmod 755 "$install_dir/lmml-router"
  fi
  if [ -n "$uninstaller" ]; then
    cp "$uninstaller" "$install_dir/lmml-uninstall"
    chmod 755 "$install_dir/lmml-uninstall"
  fi
}

install_binary_mode() {
  tarball="lmml-$VERSION-$target.tar.gz"
  tarball_url="$BASE_URL/$tarball"
  download "$tarball_url" "$TMP_DIR/$tarball"
  verify_download "$tarball" "$tarball_url"

  mkdir -p "$TMP_DIR/extract"
  tar -xzf "$TMP_DIR/$tarball" -C "$TMP_DIR/extract"
  binary=$(find "$TMP_DIR/extract" -type f -name lmml | head -n 1)
  if [ -z "$binary" ]; then
    fail "lmml binary not found in release archive"
  fi
  node_binary=$(find "$TMP_DIR/extract" -type f -name lmml-node | head -n 1)
  router_binary=$(find "$TMP_DIR/extract" -type f -name lmml-router | head -n 1)
  if [ -z "$node_binary" ]; then
    fail "lmml-node binary not found in release archive"
  fi
  if [ -z "$router_binary" ]; then
    fail "lmml-router binary not found in release archive"
  fi
  uninstaller=$(find "$TMP_DIR/extract" -type f -path '*/scripts/uninstall.sh' | head -n 1)
  install_binary_and_uninstaller "$binary" "$uninstaller" "$node_binary" "$router_binary"
}

install_source_mode() {
  if ! command -v bash >/dev/null 2>&1; then
    fail "bash is required for INSTALL_MODE=source"
  fi
  preflight_url="$BASE_URL/preflight.sh"
  download "$preflight_url" "$TMP_DIR/preflight.sh"
  LMML_INSTALL_MODE=source LMML_GPU_MODE="${LMML_GPU_MODE:-required}" bash "$TMP_DIR/preflight.sh"

  source_tarball="lmml-$VERSION-source.tar.gz"
  source_url="$BASE_URL/$source_tarball"
  download "$source_url" "$TMP_DIR/$source_tarball"
  verify_download "$source_tarball" "$source_url"

  mkdir -p "$TMP_DIR/source"
  tar -xzf "$TMP_DIR/$source_tarball" -C "$TMP_DIR/source"
  source_dir=$(find "$TMP_DIR/source" -maxdepth 5 -type f -path '*/crates/lmml-tui/Cargo.toml' -print | head -n 1)
  if [ -z "$source_dir" ]; then
    fail "lmml source tree not found in source archive"
  fi
  source_dir=$(dirname "$(dirname "$(dirname "$source_dir")")")
  (cd "$source_dir" && cargo build --release -p lmml-tui -p lmml-node -p lmml-router)
  binary="$source_dir/target/release/lmml"
  if [ ! -x "$binary" ]; then
    fail "source build completed but lmml binary was not produced"
  fi
  node_binary="$source_dir/target/release/lmml-node"
  router_binary="$source_dir/target/release/lmml-router"
  if [ ! -x "$node_binary" ]; then
    fail "source build completed but lmml-node binary was not produced"
  fi
  if [ ! -x "$router_binary" ]; then
    fail "source build completed but lmml-router binary was not produced"
  fi
  uninstaller="$source_dir/scripts/uninstall.sh"
  install_binary_and_uninstaller "$binary" "$uninstaller" "$node_binary" "$router_binary"
}

case "$INSTALL_MODE" in
  binary) install_binary_mode ;;
  source) install_source_mode ;;
esac

mkdir -p "$install_dir"

case ":$PATH:" in
  *":$install_dir:"*) ;;
  *)
    warn "$install_dir is not on your PATH."
    echo "   Add this to your shell profile:"
    echo "     export PATH=\"$install_dir:\$PATH\""
    echo "   Then restart your terminal or run: source ~/.bashrc"
    ;;
esac

"$install_dir/lmml" doctor || fail "lmml installed, but preflight checks failed." "Fix the hard prerequisites above, then run: $install_dir/lmml doctor"
"$install_dir/lmml" smoke || fail "lmml installed, but smoke check failed." "Run $install_dir/lmml smoke for details."

echo "✓ lmml $VERSION installed to $install_dir/lmml"
echo
echo "  Get started:"
echo "    lmml doctor       — check your system"
echo "    lmml              — launch the TUI"
echo "    lmml-node         — expose one machine as an LMML worker"
echo "    lmml-router       — route requests across LMML workers"

if [ "$LMML_PROFILE_HINT" = "orion-qwen35-4b-q8" ]; then
  echo
  echo "  Orion GTX 1080 Ti 11GB + Qwen3.5 4B Q8 deep profile target:"
  echo "    llama-server ctx_size:              262144 tokens"
  echo "    OpenCode compaction.reserved:       65536 tokens"
  echo "    OpenCode usable input limit:        196608 tokens"
  echo "    OpenCode output limit:              18000 tokens"
  echo "    operator compact target:            90000-120000 live prompt tokens"
  echo "    operator red zone:                  120000-170000 live prompt tokens"
  echo "    operator hard compress/reject:      170000-190000 live prompt tokens"
  echo "    OpenCode provider timeout:          7200 seconds"
  echo "    OpenCode stream chunk timeout:      2400 seconds"
  echo "    llama-server parallel slots:        1"
  echo "    recommended extra_args:             [\"--parallel\", \"1\", \"--slot-save-path\", \"\$HOME/.local/share/lmml/llama-slots\"]"
  echo "    recommended KV/cache args:          [\"-ctk\", \"q8_0\", \"-ctv\", \"q8_0\", \"--cache-ram\", \"4096\"]"
  echo
  echo "  TUI runtime profiles for Qwen3.5-4B-Q8_0.gguf:"
  echo "    orion-qwen-q8-deep:                 ctx 262144, parallel 1, 0 subagents"
  echo "    orion-qwen-q8-balanced:             ctx 262144, parallel 2, 1 subagent max"
  echo "    orion-qwen-q8-kvu-fanout4:          ctx 65536, parallel 4, q4 KV, kv-unified"
  echo "    orion-qwen-q8-kvu-fanout6:          ctx 65536, parallel 6, q4 KV, kv-unified"
  echo "    orion-qwen-q8-kvu-fanout8:          ctx 65536, parallel 8, q4 KV, kv-unified"
  echo "    5060ti-qwen4b-fanout4:              ctx 131072, parallel 4, 3 subagents max"
  echo "    5060ti-qwen4b-dual:                 ctx 262144, parallel 2, 1 subagent max"
  echo "    5060ti-qwen4b-kvu-fanout4:          ctx 73728, parallel 4, q4 KV, kv-unified"
  echo "    5060ti-qwen4b-kvu-fanout6:          ctx 73728, parallel 6, q4 KV, kv-unified"
  echo "    5060ti-qwen4b-kvu-fanout8:          ctx 73728, parallel 8, q4 KV, kv-unified"
  echo "    5070ti-qwen4b-fanout4:              ctx 131072, parallel 4, 3 subagents max"
  echo "    5070ti-qwen4b-dual:                 ctx 262144, parallel 2, 1 subagent max"
  echo "    5070ti-qwen4b-kvu-fanout4:          ctx 73728, parallel 4, q4 KV, kv-unified"
  echo "    5070ti-qwen4b-kvu-fanout6:          ctx 73728, parallel 6, q4 KV, kv-unified"
  echo "    5070ti-qwen4b-kvu-fanout8:          ctx 73728, parallel 8, q4 KV, kv-unified"
  echo "    TUI switch key:                     p on Models or Server tab"
  echo
  echo "  Notes:"
  echo "    Restart OpenCode after changing opencode.json; provider settings are loaded at session start."
  echo "    Use embedded GGUF chat templates by leaving lmml chat_template empty unless a per-model override is proven."
  echo "    Orion single-shot complex mode is one resident slot; do not spawn background subagents."
fi

if [ "$LMML_PROFILE_HINT" = "quadro-m6000-qwen35-9b-q8" ]; then
  echo
  echo "  Quadro M6000 24GB + Qwen3.5 9B Q8 profile target:"
  echo "    llama-server ctx_size:              262144 tokens"
  echo "    OpenCode compaction.reserved:       49152 tokens for 4-slot fanout"
  echo "    per-slot context at parallel 4:     65536 tokens"
  echo "    recommended subagent soft cap:      32768 tokens"
  echo "    llama-server parallel slots:        4"
  echo "    deep-run alternate reserve:         65536 tokens at parallel 1"
  echo "    Qwen thinking sampling:             temperature=0.6 top_p=0.95 top_k=20 min_p=0"
  echo "    Qwen non-thinking sampling:         temperature=0.7 top_p=0.8 top_k=20 min_p=0"
  echo "    minimum context for thinking:       128000 tokens"
  echo "    recommended extra_args:             [\"--parallel\", \"4\", \"--slot-save-path\", \"\$HOME/.local/share/lmml/llama-slots\"]"
  echo "    recommended KV/cache args:          [\"-ctk\", \"q8_0\", \"-ctv\", \"q8_0\", \"--cache-ram\", \"4096\"]"
  echo "    vision/video support:               requires matching mmproj vision encoder beside the GGUF"
  echo "    MTP support:                        supported by model, keep disabled until profiled"
  echo
  echo "  TUI runtime profiles for Qwen3.5-9B-Q8_0.gguf:"
  echo "    m6000-qwen9b-deep:                  ctx 262144, parallel 1, 0 subagents"
  echo "    m6000-qwen9b-fanout1:               ctx 262144, parallel 2, 1 subagent max"
  echo "    m6000-qwen9b-fanout2:               ctx 262144, parallel 2, 2 subagents max"
  echo "    m6000-qwen9b-fanout3:               ctx 262144, parallel 3, 3 subagents max"
  echo "    m6000-qwen9b-fanout4:               ctx 262144, parallel 4, 3 subagents max"
  echo "    m6000-qwen9b-fanout6:               ctx 262144, parallel 6, 5 subagents after validation"
  echo "    m6000-qwen9b-mtp-deep:              ctx 262144, parallel 1, MTP enabled, text-only"
  echo "    m6000-qwen9b-mtp-vision:            ctx 262144, parallel 1, MTP + mmproj vision"
  echo "    m6000-qwen9b-kvu-fanout4:           ctx 86016, parallel 4, q4 KV, kv-unified"
  echo "    m6000-qwen9b-kvu-fanout6:           ctx 86016, parallel 6, q4 KV, kv-unified"
  echo "    m6000-qwen9b-kvu-fanout8:           ctx 86016, parallel 8, q4 KV, kv-unified"
  echo "    5060ti-qwen9b-deep:                 ctx 196608, parallel 1, 0 subagents"
  echo "    5060ti-qwen9b-balanced2:            ctx 131072, parallel 2, 1 subagent max"
  echo "    5060ti-qwen9b-kvu-fanout4:          ctx 73728, parallel 4, q4 KV, kv-unified"
  echo "    5060ti-qwen9b-kvu-fanout6:          ctx 73728, parallel 6, q4 KV, kv-unified"
  echo "    5060ti-qwen9b-kvu-fanout8:          ctx 73728, parallel 8, q4 KV, kv-unified"
  echo "    5070ti-qwen9b-deep:                 ctx 196608, parallel 1, 0 subagents"
  echo "    5070ti-qwen9b-balanced2:            ctx 131072, parallel 2, 1 subagent max"
  echo "    5070ti-qwen9b-kvu-fanout4:          ctx 73728, parallel 4, q4 KV, kv-unified"
  echo "    5070ti-qwen9b-kvu-fanout6:          ctx 73728, parallel 6, q4 KV, kv-unified"
  echo "    5070ti-qwen9b-kvu-fanout8:          ctx 73728, parallel 8, q4 KV, kv-unified"
  echo "    TUI switch key:                     p on Models or Server tab"
  echo
  echo "  Fallback ladder if runtime memory pressure appears:"
  echo "    ctx_size 196608, reserved 65536, parallel 1, practical 131072"
  echo "    ctx_size 131072, reserved 32768, parallel 1, practical 98304"
fi

if [ "$LMML_PROFILE_HINT" = "qwen36-27b-q4" ] || [ "$LMML_PROFILE_HINT" = "qwen36-35b-a3b-q4" ]; then
  echo
  echo "Qwen3.6 profile hint:"
  echo
  echo "  Official open Qwen3.6 local variants:"
  echo "    Qwen3.6-27B:                       dense, 262144 context"
  echo "    Qwen3.6-35B-A3B:                   MoE, 3B active, 262144 context"
  echo
  echo "  LMML support mode:"
  echo "    model catalog:                     supported"
  echo "    built-in runtime profile:          not yet field-validated"
  echo "    recommended backend:               CUDA first, ROCm/Metal/Vulkan only after local validation"
  echo "    recommended quant for local use:   Q4-class GGUF on 24GB minimum, 32GB+ preferred"
  echo
  echo "  Notes:"
  echo "    Use embedded GGUF chat templates unless a Qwen3.6-specific template is validated."
  echo "    Start with single-slot serving before enabling fanout or kv-unified concurrency."
  echo "    See docs/llm-model-support.md after installing from source or the repo checkout."
fi

if [ "$LMML_PROFILE_HINT" = "gemma4-12b-mtp-q4km" ]; then
  echo
  echo "Gemma4 12B QAT Q4_K_M MTP profile hint:"
  echo
  echo "  Required model files:"
  echo "    main GGUF:                          Gemma4-12B-QAT-Q4_K_M.gguf"
  echo "    official QAT alternative:           gemma-4-12b-it-qat-q4_0.gguf"
  echo "    MTP draft GGUF:                     mtp-gemma-4-12B-it.gguf"
  echo
  echo "  TUI runtime profile:"
  echo "    gemma4-12b-mtp-q4km:                ctx 73728, parallel 1, MTP draft head"
  echo
  echo "  llama-server arguments emitted by profile:"
  echo "    -ngl 99 -fa on"
  echo "    -md <models>/mtp-gemma-4-12B-it.gguf --spec-type draft-mtp"
  echo "    --temp 0.6 --top-k 64 --top-p 0.9 --min-p 0.05 --repeat-penalty 1.1"
  echo
  echo "  Gemma 4 implementation notes:"
  echo "    roles:                              native system/user/assistant"
  echo "    thinking toggle:                    <|think|> in the system prompt"
  echo "    QAT guidance:                       prefer Google q4_0 GGUF where available"
  echo "    MTP:                                matching drafter required for speculative decoding"
  echo
  echo "  Put both GGUF files in ~/.local/share/lmml/models, select"
  echo "  Gemma4-12B-QAT-Q4_K_M.gguf in the TUI, then press p until"
  echo "  Profile shows gemma4-12b-mtp-q4km."
fi

if [ "$LMML_PROFILE_HINT" = "bc250-qwen35-9b-q4km-vulkan" ]; then
  echo
  echo "AMD BC-250 headless Vulkan + Qwen3.5 9B Q4_K_M profile hint:"
  echo
  echo "  Expected footprint:"
  echo "    Ubuntu Server / Debian headless:     ~6-8GB"
  echo "    llama.cpp source + built binaries:   ~1GB"
  echo "    Qwen3.5 9B Q4_K_M GGUF:              ~5.5GB"
  echo
  echo "  Recommended source install path:"
  echo "    LMML_GPU_MODE=vulkan INSTALL_MODE=source"
  echo
  echo "  Required runtime stack:"
  echo "    backend:                             Vulkan / RADV, not CUDA"
  echo "    Mesa:                                25.1+ recommended for BC-250"
  echo "    model file:                          Qwen3.5-9B-Q4_K_M.gguf"
  echo "    if your model is named differently:  rename or symlink it to Qwen3.5-9B-Q4_K_M.gguf"
  echo
  echo "  TUI runtime profile:"
  echo "    bc250-qwen9b-q4km-vulkan:            host 0.0.0.0, port 8080"
  echo "    ctx_size:                            4096"
  echo "    gpu layers:                          99"
  echo "    threads:                             6"
  echo "    parallel:                            1"
  echo
  echo "  Equivalent llama-server shape:"
  echo "    llama-server -m ~/.local/share/lmml/models/Qwen3.5-9B-Q4_K_M.gguf --host 0.0.0.0 --port 8080 -ngl 99 -fa -c 4096"
  echo
  echo "  Service guidance:"
  echo "    See docs/lan-client-install.md for a systemd unit that starts lmml/llama-server on boot."
  echo "    Exposing 0.0.0.0:8080 is for trusted LANs only; use firewall rules on shared networks."
fi
