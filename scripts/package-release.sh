#!/bin/sh
set -eu

ROOT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
CRATE_TOML="$ROOT_DIR/crates/lmml-tui/Cargo.toml"
VERSION=$(awk -F\" '/^version = / { print $2; exit }' "$CRATE_TOML")
TARGET_TRIPLE=${TARGET_TRIPLE:-$(rustc -vV | awk '/^host:/ { print $2 }')}
TARBALL="lmml-$VERSION-$TARGET_TRIPLE.tar.gz"
DIST_DIR="$ROOT_DIR/dist"
STAGE_DIR="$ROOT_DIR/target/package/lmml-$VERSION-$TARGET_TRIPLE"
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
mkdir -p "$STAGE_DIR/scripts"

cargo build --release -p lmml-tui --target "$TARGET_TRIPLE"

cp "$ROOT_DIR/target/$TARGET_TRIPLE/release/lmml" "$STAGE_DIR/lmml"
cp "$ROOT_DIR/README.md" "$STAGE_DIR/README.md"
cp "$ROOT_DIR/LICENSE" "$STAGE_DIR/LICENSE"
cp "$ROOT_DIR/scripts/install.sh" "$STAGE_DIR/scripts/install.sh"
cp "$ROOT_DIR/scripts/uninstall.sh" "$STAGE_DIR/scripts/uninstall.sh"
cp "$ROOT_DIR/scripts/install.sh" "$DIST_DIR/install.sh"
cp "$ROOT_DIR/scripts/uninstall.sh" "$DIST_DIR/uninstall.sh"
printf '%s\n' "$VERSION" > "$DIST_DIR/latest"

cat > "$STAGE_DIR/RELEASE-METADATA" <<EOF
version=$VERSION
target=$TARGET_TRIPLE
git_commit=$(git -C "$ROOT_DIR" rev-parse HEAD 2>/dev/null || echo unknown)
rustc=$(rustc --version)
source_date_epoch=$SOURCE_DATE_EPOCH
EOF

chmod 755 "$STAGE_DIR/lmml" "$STAGE_DIR/scripts/install.sh" "$STAGE_DIR/scripts/uninstall.sh"
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
if [ -n "$ALIAS_TARBALL" ]; then
  update_checksums "$ALIAS_TARBALL"
fi

if command -v sha256sum >/dev/null 2>&1; then
  CHECKSUM=$(cd "$DIST_DIR" && sha256sum "$TARBALL" | awk '{ print $1 }')
else
  CHECKSUM=$(cd "$DIST_DIR" && shasum -a 256 "$TARBALL" | awk '{ print $1 }')
fi

echo "tarball: $DIST_DIR/$TARBALL"
echo "sha256:  $CHECKSUM"
