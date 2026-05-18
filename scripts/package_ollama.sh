#!/bin/bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODELFILE="${ROOT_DIR}/Modelfile.trained"
MODEL_NAME="${1:-rem-coder-trained}"
QUANT_NAME="${2:-q4_k_m}"
GGUF_FILE="${ROOT_DIR}/models/rem-coder-gguf/rem-coder-${QUANT_NAME}.gguf"

if ! command -v ollama >/dev/null 2>&1; then
  echo "Ollama not found. Install from https://ollama.com"
  exit 1
fi

if [ ! -f "${GGUF_FILE}" ]; then
  echo "GGUF file not found: ${GGUF_FILE}"
  echo "Run scripts/export_gguf.sh with QUANT_LIST including ${QUANT_NAME}."
  exit 1
fi

TMP_MODELFILE="${ROOT_DIR}/models/rem-coder-gguf/Modelfile.${QUANT_NAME}.tmp"
sed "s#^FROM .*#FROM ./models/rem-coder-gguf/rem-coder-${QUANT_NAME}.gguf#" "${MODELFILE}" > "${TMP_MODELFILE}"

ollama create "${MODEL_NAME}" -f "${TMP_MODELFILE}"
rm -f "${TMP_MODELFILE}"

echo "Created Ollama model: ${MODEL_NAME}"
echo "Run with: ollama run ${MODEL_NAME}"
