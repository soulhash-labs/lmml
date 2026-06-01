#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
FAKE_BIN=$(mktemp -d "${TMPDIR:-/tmp}/lmml-preflight-bin.XXXXXX")

cleanup() {
  rm -rf "$FAKE_BIN"
}
trap cleanup EXIT INT TERM

link_tool() {
  tool=$1
  if command -v "$tool" >/dev/null 2>&1; then
    ln -s "$(command -v "$tool")" "$FAKE_BIN/$tool"
  fi
}

link_one_of() {
  for tool in "$@"; do
    if command -v "$tool" >/dev/null 2>&1; then
      ln -s "$(command -v "$tool")" "$FAKE_BIN/$tool"
      return 0
    fi
  done
  echo "missing required host tool for preflight fixture: $*" >&2
  exit 1
}

for tool in awk bash df grep head sed uname; do
  link_tool "$tool"
done
link_one_of curl wget
link_one_of g++ clang++
link_one_of cmake
link_one_of git

if PATH="$FAKE_BIN" LMML_INSTALL_MODE=binary LMML_GPU_MODE=cpu-only bash "$ROOT_DIR/scripts/preflight.sh" >/tmp/lmml-preflight-binary.out; then
  :
else
  cat /tmp/lmml-preflight-binary.out >&2
  echo "binary cpu-only preflight should pass without Rust" >&2
  exit 1
fi

if PATH="$FAKE_BIN" LMML_INSTALL_MODE=source LMML_GPU_MODE=cpu-only bash "$ROOT_DIR/scripts/preflight.sh" >/tmp/lmml-preflight-source.out 2>&1; then
  cat /tmp/lmml-preflight-source.out >&2
  echo "source preflight should fail without Rust in fixture PATH" >&2
  exit 1
fi
grep -q "rustc not found" /tmp/lmml-preflight-source.out

if PATH="$FAKE_BIN" LMML_INSTALL_MODE=binary bash "$ROOT_DIR/scripts/preflight.sh" >/tmp/lmml-preflight-gpu.out 2>&1; then
  cat /tmp/lmml-preflight-gpu.out >&2
  echo "GPU-required preflight should fail when GPU tools are absent" >&2
  exit 1
fi
grep -q "GPU acceleration failure" /tmp/lmml-preflight-gpu.out

echo "preflight fixtures passed"
