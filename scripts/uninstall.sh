#!/bin/sh
set -eu

if [ "${PREFIX:-}" ]; then
  bin_dir="$PREFIX/bin"
elif [ "$(id -u)" -eq 0 ]; then
  bin_dir="/usr/local/bin"
else
  bin_dir="$HOME/.local/bin"
fi
binary="$bin_dir/lmml"
node_binary="$bin_dir/lmml-node"
router_binary="$bin_dir/lmml-router"
uninstaller="$bin_dir/lmml-uninstall"

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
  rm -f "$node_binary"
  rm -f "$router_binary"
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
