#!/bin/sh
set -eu

BASE_URL=${BASE_URL:-http://127.0.0.1:8000}
INSTALL_HOME=${INSTALL_HOME:-$(mktemp -d "${TMPDIR:-/tmp}/lmml-clean-home.XXXXXX")}
export HOME="$INSTALL_HOME"
export XDG_CONFIG_HOME="$HOME/.config"
export XDG_DATA_HOME="$HOME/.local/share"
export PATH="$HOME/.local/bin:$PATH"

if [ -e "$HOME/.config/lmml" ]; then
  echo "clean install smoke requires no existing $HOME/.config/lmml" >&2
  exit 1
fi

installer="$HOME/install.sh"
if command -v curl >/dev/null 2>&1; then
  curl -fsSL "$BASE_URL/install.sh" -o "$installer"
elif command -v wget >/dev/null 2>&1; then
  wget -q "$BASE_URL/install.sh" -O "$installer"
else
  echo "curl or wget is required for clean install smoke" >&2
  exit 1
fi

BASE_URL="$BASE_URL" sh "$installer"

"$HOME/.local/bin/lmml" doctor

timeout 5s "$HOME/.local/bin/lmml" smoke

LMML_ASSUME_YES=1 "$HOME/.local/bin/lmml-uninstall"

if [ -e "$HOME/.local/bin/lmml" ]; then
  echo "lmml binary still exists after uninstall" >&2
  exit 1
fi

echo "clean install smoke passed"
