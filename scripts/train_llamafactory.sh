#!/bin/bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CFG_FILE="${ROOT_DIR}/config/llamafactory_qlora.yaml"

if ! command -v llamafactory-cli >/dev/null 2>&1; then
  echo "llamafactory-cli not found. Install with: pip install llamafactory"
  exit 1
fi

if [ ! -f "${ROOT_DIR}/data/train.jsonl" ]; then
  echo "Missing data/train.jsonl. Run: python scripts/prepare_data.py"
  exit 1
fi

echo "Starting LlamaFactory QLoRA training"
llamafactory-cli train "${CFG_FILE}"
echo "LlamaFactory training complete"
