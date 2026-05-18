#!/bin/bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MERGED_DIR="${ROOT_DIR}/models/rem-coder-merged"
GGUF_DIR="${ROOT_DIR}/models/rem-coder-gguf"
QUANT_LIST="${QUANT_LIST:-q4_k_m}"

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

for quant in ${QUANT_LIST}; do
  out_file="${GGUF_DIR}/rem-coder-${quant}.gguf"
  echo "Quantizing ${quant} -> ${out_file}"
  "${LLAMA_CPP_PATH}/build/bin/llama-quantize" \
    "${GGUF_DIR}/rem-coder-f16.gguf" \
    "${out_file}" \
    "${quant}"
done

echo "GGUF export complete in ${GGUF_DIR}"
