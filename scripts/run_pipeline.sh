#!/bin/bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CONFIG_FILE="${ROOT_DIR}/config/config.yaml"

BASE_OLLAMA_MODEL="${1:-deepseek-coder:1.3b}"
TRAINED_OLLAMA_MODEL="${2:-rem-coder-trained}"
SKIP_DEPS="${SKIP_DEPS:-1}"
SKIP_BASELINE_IF_EXISTS="${SKIP_BASELINE_IF_EXISTS:-1}"
RUN_ID="${RUN_ID:-$(date +%Y%m%d-%H%M%S)}"
RUN_DIR="${ROOT_DIR}/models/experiments/${RUN_ID}"

mkdir -p "${RUN_DIR}"

echo "=== REM LLM 7-Phase Training Pipeline ==="
echo "Run ID: ${RUN_ID}"
echo "Run directory: ${RUN_DIR}"

echo "[1/7] Install minimal Python dependencies"
if [ "${SKIP_DEPS}" = "1" ]; then
  echo "Skipping dependency install (SKIP_DEPS=1)"
else
  python -m pip install -r "${ROOT_DIR}/requirements.txt"
fi

echo "[2/7] Prepare datasets (train/val/eval)"
python "${ROOT_DIR}/scripts/prepare_data.py" --config "${CONFIG_FILE}"

echo "[3/7] Baseline evaluation"
if [ "${SKIP_BASELINE_IF_EXISTS}" = "1" ] && [ -f "${ROOT_DIR}/models/evals/baseline.json" ]; then
  echo "Skipping baseline evaluation (cached report exists)"
else
  python "${ROOT_DIR}/scripts/evaluate_model.py" \
    --config "${CONFIG_FILE}" \
    --model "${BASE_OLLAMA_MODEL}" \
    --report "${ROOT_DIR}/models/evals/baseline.json"
fi

python "${ROOT_DIR}/scripts/evaluate_exec.py" \
  --config "${CONFIG_FILE}" \
  --model "${BASE_OLLAMA_MODEL}" \
  --report "${ROOT_DIR}/models/evals/baseline_exec.json"

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

python "${ROOT_DIR}/scripts/evaluate_exec.py" \
  --config "${CONFIG_FILE}" \
  --model "${TRAINED_OLLAMA_MODEL}" \
  --report "${ROOT_DIR}/models/evals/post_train_exec.json"

python "${ROOT_DIR}/scripts/compare_reports.py" \
  --baseline "${ROOT_DIR}/models/evals/baseline.json" \
  --post "${ROOT_DIR}/models/evals/post_train.json" \
  --baseline-exec "${ROOT_DIR}/models/evals/baseline_exec.json" \
  --post-exec "${ROOT_DIR}/models/evals/post_train_exec.json"

python "${ROOT_DIR}/scripts/write_run_metadata.py" \
  --run-id "${RUN_ID}" \
  --base-model "${BASE_OLLAMA_MODEL}" \
  --trained-model "${TRAINED_OLLAMA_MODEL}" \
  --baseline-report "${ROOT_DIR}/models/evals/baseline.json" \
  --post-report "${ROOT_DIR}/models/evals/post_train.json" \
  --baseline-exec-report "${ROOT_DIR}/models/evals/baseline_exec.json" \
  --post-exec-report "${ROOT_DIR}/models/evals/post_train_exec.json" \
  --config-file "${CONFIG_FILE}"

echo "=== Pipeline complete ==="
