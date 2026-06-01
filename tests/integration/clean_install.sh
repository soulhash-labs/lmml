#!/bin/sh
set -eu

BASE_URL=${BASE_URL:-http://127.0.0.1:8000}
ROOT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
INSTALL_HOME=${INSTALL_HOME:-$(mktemp -d "${TMPDIR:-/tmp}/lmml-clean-home.XXXXXX")}
export HOME="$INSTALL_HOME"
export XDG_CONFIG_HOME="$HOME/.config"
export XDG_DATA_HOME="$HOME/.local/share"
export PATH="$HOME/.local/bin:$PATH"

if [ -e "$HOME/.config/lmml" ]; then
  echo "clean install smoke requires no existing $HOME/.config/lmml" >&2
  exit 1
fi

BASE_URL="$BASE_URL" sh "$ROOT_DIR/scripts/install.sh"

"$HOME/.local/bin/lmml" doctor

timeout 5s "$HOME/.local/bin/lmml" smoke

LMML_ASSUME_YES=1 sh "$ROOT_DIR/scripts/uninstall.sh"

if [ -e "$HOME/.local/bin/lmml" ]; then
  echo "lmml binary still exists after uninstall" >&2
  exit 1
fi

echo "clean install smoke passed"
