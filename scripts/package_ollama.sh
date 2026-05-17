#!/bin/bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODELFILE="${ROOT_DIR}/Modelfile.trained"
MODEL_NAME="${1:-rem-coder-trained}"

if ! command -v ollama >/dev/null 2>&1; then
  echo "Ollama not found. Install from https://ollama.com"
  exit 1
fi

if [ ! -f "${ROOT_DIR}/models/rem-coder-gguf/rem-coder-q4_k_m.gguf" ]; then
  echo "GGUF file not found. Run scripts/export_gguf.sh first."
  exit 1
fi

ollama create "${MODEL_NAME}" -f "${MODELFILE}"
echo "Created Ollama model: ${MODEL_NAME}"
echo "Run with: ollama run ${MODEL_NAME}"
