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
      esac
      ;;
    *)
      case "$1" in
        compiler) echo "sudo apt install build-essential" ;;
        cmake) echo "sudo apt install cmake" ;;
        git) echo "sudo apt install git" ;;
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

  command -v nvcc >/dev/null 2>&1 || warn "CUDA not found; will build CPU-only"
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
  ""|orion-qwen35-4b-q8|quadro-m6000-qwen35-9b-q8) ;;
  *) fail "Unsupported LMML_PROFILE_HINT=$LMML_PROFILE_HINT" "Supported hints: orion-qwen35-4b-q8, quadro-m6000-qwen35-9b-q8." ;;
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
  mkdir -p "$install_dir"
  cp "$binary" "$install_dir/lmml"
  chmod 755 "$install_dir/lmml"
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
  uninstaller=$(find "$TMP_DIR/extract" -type f -path '*/scripts/uninstall.sh' | head -n 1)
  install_binary_and_uninstaller "$binary" "$uninstaller"
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
  (cd "$source_dir" && cargo build --release -p lmml-tui)
  binary="$source_dir/target/release/lmml"
  if [ ! -x "$binary" ]; then
    fail "source build completed but lmml binary was not produced"
  fi
  uninstaller="$source_dir/scripts/uninstall.sh"
  install_binary_and_uninstaller "$binary" "$uninstaller"
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
  echo "    5070ti-qwen4b-fanout4:              ctx 131072, parallel 4, 3 subagents max"
  echo "    5070ti-qwen4b-dual:                 ctx 262144, parallel 2, 1 subagent max"
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
  echo "    m6000-qwen9b-fanout4:               ctx 262144, parallel 4, 3 subagents max"
  echo "    m6000-qwen9b-fanout6:               ctx 262144, parallel 6, 5 subagents after validation"
  echo "    5070ti-qwen9b-deep:                 ctx 196608, parallel 1, 0 subagents"
  echo "    5070ti-qwen9b-balanced2:            ctx 131072, parallel 2, 1 subagent max"
  echo "    TUI switch key:                     p on Models or Server tab"
  echo
  echo "  Fallback ladder if runtime memory pressure appears:"
  echo "    ctx_size 196608, reserved 65536, parallel 1, practical 131072"
  echo "    ctx_size 131072, reserved 32768, parallel 1, practical 98304"
fi
