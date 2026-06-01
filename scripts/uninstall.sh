#!/bin/sh
set -eu

if [ "${PREFIX:-}" ]; then
  binary="$PREFIX/bin/lmml"
  uninstaller="$PREFIX/bin/lmml-uninstall"
elif [ "$(id -u)" -eq 0 ]; then
  binary="/usr/local/bin/lmml"
  uninstaller="/usr/local/bin/lmml-uninstall"
else
  binary="$HOME/.local/bin/lmml"
  uninstaller="$HOME/.local/bin/lmml-uninstall"
fi

prompt_yes() {
  question=$1
  if [ "${LMML_ASSUME_YES:-}" = "1" ]; then
    return 0
  fi
  printf '%s [y/N] ' "$question"
  read answer
  case "$answer" in
    y|Y|yes|YES) return 0 ;;
    *) return 1 ;;
  esac
}

if prompt_yes "Uninstall lmml from $binary?"; then
  rm -f "$binary"
  rm -f "$uninstaller"
else
  echo "Uninstall cancelled."
  exit 0
fi

config_dir="${XDG_CONFIG_HOME:-$HOME/.config}/lmml"
data_dir="${XDG_DATA_HOME:-$HOME/.local/share}/lmml"

echo "Config directory: $config_dir"
if [ -d "$config_dir" ] && prompt_yes "Delete lmml config directory?"; then
  rm -rf "$config_dir"
fi

echo "Data directory: $data_dir"
if [ -d "$data_dir" ] && prompt_yes "Delete lmml data directory?"; then
  rm -rf "$data_dir"
fi

echo "✓ lmml uninstalled."
