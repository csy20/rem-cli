#!/usr/bin/env bash
set -euo pipefail

# Install rem from this source tree via cargo.
# For prebuilt binaries, use the repo-root install.sh instead:
#   curl -fsSL https://raw.githubusercontent.com/csy20/rem-cli/main/install.sh | bash

BIN_NAME="rem"
INSTALL_ROOT="${CARGO_HOME:-${HOME}/.cargo}"
INSTALL_DIR="${INSTALL_ROOT}/bin"

if command -v cargo &>/dev/null; then
  echo "  Installing ${BIN_NAME} from source..."
  cargo install --path "$(dirname "$0")" --root "${INSTALL_ROOT}" --force
  echo "  Installed ${BIN_NAME} to ${INSTALL_DIR}/${BIN_NAME}"
  echo "  Run \`${BIN_NAME} --help\` to get started."
else
  echo "  Cargo not found. Install Rust first: https://rustup.rs"
  exit 1
fi
