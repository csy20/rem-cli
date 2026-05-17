#!/bin/bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo "=== REM LLM Training (CPU Mode) ==="

MODEL_NAME="deepseek-coder"
MODEL_SIZE="1.3b"
MODEL_TAG="${MODEL_NAME}:${MODEL_SIZE}"

echo "Step 1: Verifying base model..."
if ! ollama list | grep -q "${MODEL_TAG}"; then
    echo "Pulling ${MODEL_TAG}..."
    ollama pull ${MODEL_TAG}
fi

echo "Step 2: Creating REM model from Modelfile..."
ollama create rem-coder -f "${ROOT_DIR}/Modelfile"

echo "Step 3: Testing REM model..."
echo ""
echo "Test 1: Write a Python function to reverse a string"
echo "Write a Python function to reverse a string" | ollama run rem-coder

echo ""
echo "=== Training Complete ==="
echo "Model: rem-coder"
echo "Run with: ollama run rem-coder"
