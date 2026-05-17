#!/bin/bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MERGED_DIR="${ROOT_DIR}/models/rem-coder-merged"
GGUF_DIR="${ROOT_DIR}/models/rem-coder-gguf"

if [ ! -d "${MERGED_DIR}" ]; then
  echo "Merged model not found at ${MERGED_DIR}"
  echo "Run: python scripts/merge_adapter.py"
  exit 1
fi

if [ -z "${LLAMA_CPP_PATH:-}" ]; then
  echo "Set LLAMA_CPP_PATH to your llama.cpp directory before running."
  exit 1
fi

mkdir -p "${GGUF_DIR}"

python "${LLAMA_CPP_PATH}/convert_hf_to_gguf.py" \
  "${MERGED_DIR}" \
  --outfile "${GGUF_DIR}/rem-coder-f16.gguf" \
  --outtype f16

"${LLAMA_CPP_PATH}/build/bin/llama-quantize" \
  "${GGUF_DIR}/rem-coder-f16.gguf" \
  "${GGUF_DIR}/rem-coder-q4_k_m.gguf" \
  q4_k_m

echo "GGUF export complete: ${GGUF_DIR}/rem-coder-q4_k_m.gguf"
