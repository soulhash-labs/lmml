#!/bin/sh
set -eu

ROOT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)

sh -n "$ROOT_DIR/scripts/install.sh"
sh -n "$ROOT_DIR/scripts/package-release.sh"
sh -n "$ROOT_DIR/tests/integration/clean_install.sh"
sh -n "$ROOT_DIR/tests/integration/signed_checksums.sh"

bash -n "$ROOT_DIR/scripts/preflight.sh"
bash -n "$ROOT_DIR/tests/integration/preflight.sh"

echo "script syntax checks passed"
