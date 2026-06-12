#!/usr/bin/env bash
set -euo pipefail

BIN_NAME="rem"
REPO="rem-llm/rem-cli"
INSTALL_DIR="${HOME}/.cargo/bin"

if command -v cargo &>/dev/null; then
  echo "  Installing ${BIN_NAME} from source..."
  cargo install --path "$(dirname "$0")" --root "${INSTALL_DIR}/.."
  echo "  Installed ${BIN_NAME} to ${INSTALL_DIR}/${BIN_NAME}"
  echo "  Run \`${BIN_NAME} --help\` to get started."
else
  echo "  Cargo not found. Install Rust first: https://rustup.rs"
  exit 1
fi
