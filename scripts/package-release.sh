#!/bin/sh
set -eu

ROOT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
CRATE_TOML="$ROOT_DIR/crates/lmml-tui/Cargo.toml"
VERSION=$(awk -F\" '/^version = / { print $2; exit }' "$CRATE_TOML")
TARGET_TRIPLE=${TARGET_TRIPLE:-$(rustc -vV | awk '/^host:/ { print $2 }')}
TARBALL="lmml-$VERSION-$TARGET_TRIPLE.tar.gz"
SOURCE_TARBALL="lmml-$VERSION-source.tar.gz"
DIST_DIR="$ROOT_DIR/dist"
STAGE_DIR="$ROOT_DIR/target/package/lmml-$VERSION-$TARGET_TRIPLE"
SOURCE_STAGE_DIR="$ROOT_DIR/target/package/lmml-$VERSION-source"
SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-$(git -C "$ROOT_DIR" log -1 --format=%ct 2>/dev/null || date +%s)}

if [ -z "$VERSION" ]; then
  echo "failed to read version from $CRATE_TOML" >&2
  exit 1
fi
if ! tar --version 2>/dev/null | grep -q 'GNU tar'; then
  echo "GNU tar is required for reproducible release archives." >&2
  echo "Install GNU tar, then rerun scripts/package-release.sh." >&2
  exit 1
fi

mkdir -p "$DIST_DIR"
rm -rf "$STAGE_DIR"
rm -rf "$SOURCE_STAGE_DIR"
mkdir -p "$STAGE_DIR/scripts"

cargo build --release -p lmml-tui -p lmml-node -p lmml-router --target "$TARGET_TRIPLE"

cp "$ROOT_DIR/target/$TARGET_TRIPLE/release/lmml" "$STAGE_DIR/lmml"
cp "$ROOT_DIR/target/$TARGET_TRIPLE/release/lmml-node" "$STAGE_DIR/lmml-node"
cp "$ROOT_DIR/target/$TARGET_TRIPLE/release/lmml-router" "$STAGE_DIR/lmml-router"
cp "$ROOT_DIR/README.md" "$STAGE_DIR/README.md"
cp "$ROOT_DIR/LICENSE" "$STAGE_DIR/LICENSE"
cp "$ROOT_DIR/scripts/install.sh" "$STAGE_DIR/scripts/install.sh"
cp "$ROOT_DIR/scripts/preflight.sh" "$STAGE_DIR/scripts/preflight.sh"
cp "$ROOT_DIR/scripts/uninstall.sh" "$STAGE_DIR/scripts/uninstall.sh"
cp "$ROOT_DIR/scripts/install.sh" "$DIST_DIR/install.sh"
cp "$ROOT_DIR/scripts/preflight.sh" "$DIST_DIR/preflight.sh"
cp "$ROOT_DIR/scripts/uninstall.sh" "$DIST_DIR/uninstall.sh"
printf '%s\n' "$VERSION" > "$DIST_DIR/latest"

cat > "$STAGE_DIR/RELEASE-METADATA" <<EOF
version=$VERSION
target=$TARGET_TRIPLE
git_commit=$(git -C "$ROOT_DIR" rev-parse HEAD 2>/dev/null || echo unknown)
rustc=$(rustc --version)
source_date_epoch=$SOURCE_DATE_EPOCH
EOF

chmod 755 "$STAGE_DIR/lmml" "$STAGE_DIR/lmml-node" "$STAGE_DIR/lmml-router" "$STAGE_DIR/scripts/install.sh" "$STAGE_DIR/scripts/preflight.sh" "$STAGE_DIR/scripts/uninstall.sh"
chmod 644 "$STAGE_DIR/README.md" "$STAGE_DIR/LICENSE" "$STAGE_DIR/RELEASE-METADATA"
find "$STAGE_DIR" -type d -exec chmod 755 {} +

(
  cd "$ROOT_DIR/target/package"
  tar_tmp="$DIST_DIR/$TARBALL.tar"
  tar --sort=name \
    --owner=0 --group=0 --numeric-owner \
    --mtime="@$SOURCE_DATE_EPOCH" \
    -cf "$tar_tmp" "lmml-$VERSION-$TARGET_TRIPLE"
  gzip -n -c "$tar_tmp" > "$DIST_DIR/$TARBALL"
  rm -f "$tar_tmp"
)

mkdir -p "$SOURCE_STAGE_DIR"
(
  cd "$ROOT_DIR"
  tar --exclude='./.git' \
    --exclude='./target' \
    --exclude='./dist' \
    --exclude='./.planning' \
    --exclude='*.swp' \
    --exclude='.*.swp' \
    --exclude='*.kate-swp' \
    --exclude='.*.kate-swp' \
    -cf - .
) | (
  cd "$SOURCE_STAGE_DIR"
  tar -xf -
)
cat > "$SOURCE_STAGE_DIR/RELEASE-METADATA" <<EOF
version=$VERSION
target=source
git_commit=$(git -C "$ROOT_DIR" rev-parse HEAD 2>/dev/null || echo unknown)
rustc=$(rustc --version)
source_date_epoch=$SOURCE_DATE_EPOCH
EOF
find "$SOURCE_STAGE_DIR" -type d -exec chmod 755 {} +
find "$SOURCE_STAGE_DIR" -type f -exec chmod 644 {} +
chmod 755 "$SOURCE_STAGE_DIR/scripts/install.sh" "$SOURCE_STAGE_DIR/scripts/preflight.sh" "$SOURCE_STAGE_DIR/scripts/uninstall.sh"

(
  cd "$ROOT_DIR/target/package"
  source_tmp="$DIST_DIR/$SOURCE_TARBALL.tar"
  tar --sort=name \
    --owner=0 --group=0 --numeric-owner \
    --mtime="@$SOURCE_DATE_EPOCH" \
    -cf "$source_tmp" "lmml-$VERSION-source"
  gzip -n -c "$source_tmp" > "$DIST_DIR/$SOURCE_TARBALL"
  rm -f "$source_tmp"
)

case "$TARGET_TRIPLE" in
  x86_64-unknown-linux-gnu) ALIAS_TARBALL="lmml-$VERSION-x86_64-linux.tar.gz" ;;
  aarch64-unknown-linux-gnu) ALIAS_TARBALL="lmml-$VERSION-aarch64-linux.tar.gz" ;;
  x86_64-apple-darwin) ALIAS_TARBALL="lmml-$VERSION-x86_64-macos.tar.gz" ;;
  aarch64-apple-darwin) ALIAS_TARBALL="lmml-$VERSION-aarch64-macos.tar.gz" ;;
  *) ALIAS_TARBALL="" ;;
esac

if [ -n "$ALIAS_TARBALL" ]; then
  cp "$DIST_DIR/$TARBALL" "$DIST_DIR/$ALIAS_TARBALL"
fi

update_checksums() {
  sums="$DIST_DIR/SHA256SUMS"
  tmp="$DIST_DIR/SHA256SUMS.tmp"
  touch "$sums"
  awk -v file="$1" '$2 != file { print }' "$sums" > "$tmp"
  if command -v sha256sum >/dev/null 2>&1; then
    (cd "$DIST_DIR" && sha256sum "$1") >> "$tmp"
  else
    (cd "$DIST_DIR" && shasum -a 256 "$1") >> "$tmp"
  fi
  sort -k2,2 "$tmp" -o "$tmp"
  mv "$tmp" "$sums"
}

update_checksums "$TARBALL"
update_checksums "$SOURCE_TARBALL"
if [ -n "$ALIAS_TARBALL" ]; then
  update_checksums "$ALIAS_TARBALL"
fi

rm -f "$DIST_DIR/SHA256SUMS.minisig"
if [ "${LMML_SIGN_CHECKSUMS:-0}" = "1" ]; then
  if ! command -v minisign >/dev/null 2>&1; then
    echo "LMML_SIGN_CHECKSUMS=1 requires minisign on PATH." >&2
    exit 1
  fi
  if [ -z "${LMML_MINISIGN_SECRET_KEY_FILE:-}" ]; then
    echo "LMML_SIGN_CHECKSUMS=1 requires LMML_MINISIGN_SECRET_KEY_FILE." >&2
    exit 1
  fi
  minisign -Sm "$DIST_DIR/SHA256SUMS" -s "$LMML_MINISIGN_SECRET_KEY_FILE" -x "$DIST_DIR/SHA256SUMS.minisig"
fi

if command -v sha256sum >/dev/null 2>&1; then
  CHECKSUM=$(cd "$DIST_DIR" && sha256sum "$TARBALL" | awk '{ print $1 }')
else
  CHECKSUM=$(cd "$DIST_DIR" && shasum -a 256 "$TARBALL" | awk '{ print $1 }')
fi

echo "tarball: $DIST_DIR/$TARBALL"
echo "sha256:  $CHECKSUM"
echo "source:  $DIST_DIR/$SOURCE_TARBALL"
