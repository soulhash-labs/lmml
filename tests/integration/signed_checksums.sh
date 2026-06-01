#!/bin/sh
set -eu

ROOT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/lmml-signed-checksums.XXXXXX")
FAKE_BIN="$TMP_DIR/bin"

cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT INT TERM

mkdir -p "$FAKE_BIN"

link_tool() {
  tool=$1
  if command -v "$tool" >/dev/null 2>&1; then
    ln -s "$(command -v "$tool")" "$FAKE_BIN/$tool"
  fi
}

for tool in awk basename cat chmod cp dirname find grep head id mkdir mktemp rm sed tar tr uname; do
  link_tool "$tool"
done
link_tool sh

cat > "$FAKE_BIN/curl" <<'EOF'
#!/bin/sh
set -eu
url=
out=
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o)
      out=$2
      shift 2
      ;;
    -*)
      shift
      ;;
    *)
      url=$1
      shift
      ;;
  esac
done
case "$url" in
  */latest) printf '0.1.0\n' > "$out" ;;
  */SHA256SUMS) printf 'abcd  lmml-0.1.0-x86_64-unknown-linux-gnu.tar.gz\n' > "$out" ;;
  */SHA256SUMS.minisig)
    if [ "${FAKE_NO_SIG:-0}" = "1" ]; then
      exit 22
    fi
    printf 'fake signature\n' > "$out"
    ;;
  *) printf 'unexpected url: %s\n' "$url" >&2; exit 22 ;;
esac
EOF
chmod 755 "$FAKE_BIN/curl"

cat > "$FAKE_BIN/minisign" <<'EOF'
#!/bin/sh
case " $* " in
  *" -P "*|*" -p "*) exit 0 ;;
  *) printf 'minisign should not be called without a configured public key\n' >&2; exit 1 ;;
esac
EOF
chmod 755 "$FAKE_BIN/minisign"

if PATH="$FAKE_BIN" LMML_CHECKSUM_VERIFY=invalid sh "$ROOT_DIR/scripts/install.sh" >"$TMP_DIR/invalid.out" 2>&1; then
  cat "$TMP_DIR/invalid.out" >&2
  echo "invalid signed checksum mode should fail" >&2
  exit 1
fi
if ! grep -q "Unsupported LMML_CHECKSUM_VERIFY" "$TMP_DIR/invalid.out"; then
  cat "$TMP_DIR/invalid.out" >&2
  echo "invalid signed checksum mode failure was unclear" >&2
  exit 1
fi

if PATH="$FAKE_BIN" LMML_CHECKSUM_VERIFY=required sh "$ROOT_DIR/scripts/install.sh" >"$TMP_DIR/required.out" 2>&1; then
  cat "$TMP_DIR/required.out" >&2
  echo "required signed checksum verification should fail without a public key" >&2
  exit 1
fi
if ! grep -q "no minisign public key" "$TMP_DIR/required.out"; then
  cat "$TMP_DIR/required.out" >&2
  echo "required signed checksum failure did not explain missing public key" >&2
  exit 1
fi

if PATH="$FAKE_BIN" FAKE_NO_SIG=1 LMML_CHECKSUM_VERIFY=required LMML_MINISIGN_PUBLIC_KEY=RWfake sh "$ROOT_DIR/scripts/install.sh" >"$TMP_DIR/no-sig.out" 2>&1; then
  cat "$TMP_DIR/no-sig.out" >&2
  echo "required signed checksum verification should fail when signature is missing" >&2
  exit 1
fi
if ! grep -q "SHA256SUMS.minisig was not available" "$TMP_DIR/no-sig.out"; then
  cat "$TMP_DIR/no-sig.out" >&2
  echo "missing signature failure was unclear" >&2
  exit 1
fi

rm -f "$FAKE_BIN/minisign"
if PATH="$FAKE_BIN" LMML_CHECKSUM_VERIFY=required LMML_MINISIGN_PUBLIC_KEY=RWfake sh "$ROOT_DIR/scripts/install.sh" >"$TMP_DIR/no-minisign.out" 2>&1; then
  cat "$TMP_DIR/no-minisign.out" >&2
  echo "required signed checksum verification should fail when minisign is missing" >&2
  exit 1
fi
if ! grep -q "minisign is required" "$TMP_DIR/no-minisign.out"; then
  cat "$TMP_DIR/no-minisign.out" >&2
  echo "missing minisign failure was unclear" >&2
  exit 1
fi

echo "signed checksum installer fixtures passed"
