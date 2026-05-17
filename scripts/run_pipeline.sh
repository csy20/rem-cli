#!/bin/bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CONFIG_FILE="${ROOT_DIR}/config/config.yaml"

BASE_OLLAMA_MODEL="${1:-deepseek-coder:1.3b}"
TRAINED_OLLAMA_MODEL="${2:-rem-coder-trained}"

echo "=== REM LLM 7-Phase Training Pipeline ==="

echo "[1/7] Install minimal Python dependencies"
python -m pip install -r "${ROOT_DIR}/requirements.txt"

echo "[2/7] Prepare datasets (train/val/eval)"
python "${ROOT_DIR}/scripts/prepare_data.py" --config "${CONFIG_FILE}"

echo "[3/7] Baseline evaluation"
python "${ROOT_DIR}/scripts/evaluate_model.py" \
  --config "${CONFIG_FILE}" \
  --model "${BASE_OLLAMA_MODEL}" \
  --report "${ROOT_DIR}/models/evals/baseline.json"

echo "[4/7] QLoRA training (Unsloth)"
python "${ROOT_DIR}/scripts/train_unsloth.py" --config "${CONFIG_FILE}"

echo "[5/7] Merge adapter into base model"
python "${ROOT_DIR}/scripts/merge_adapter.py" --config "${CONFIG_FILE}"

echo "[6/7] Export GGUF + package in Ollama"
bash "${ROOT_DIR}/scripts/export_gguf.sh"
bash "${ROOT_DIR}/scripts/package_ollama.sh" "${TRAINED_OLLAMA_MODEL}"

echo "[7/7] Post-train evaluation + report comparison"
python "${ROOT_DIR}/scripts/evaluate_model.py" \
  --config "${CONFIG_FILE}" \
  --model "${TRAINED_OLLAMA_MODEL}" \
  --report "${ROOT_DIR}/models/evals/post_train.json"

python "${ROOT_DIR}/scripts/compare_reports.py" \
  --baseline "${ROOT_DIR}/models/evals/baseline.json" \
  --post "${ROOT_DIR}/models/evals/post_train.json"

echo "=== Pipeline complete ==="
