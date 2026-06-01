#!/bin/sh
set -eu

BASE_URL=${BASE_URL:-https://github.com/YOUR_ORG/lmml/releases/latest}
VERSION=${VERSION:-}
INSTALL_MODE=${INSTALL_MODE:-binary}
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

if [ -z "$VERSION" ]; then
  download "$BASE_URL/latest" "$TMP_DIR/latest"
  VERSION=$(sed -n '1p' "$TMP_DIR/latest" | tr -d '[:space:]')
fi

sums_url="$BASE_URL/SHA256SUMS"
download "$sums_url" "$TMP_DIR/SHA256SUMS"

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
