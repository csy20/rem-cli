# REM LLM - Coding Training Pipeline

This project trains a coding assistant model named `rem-coder` using a 7-phase workflow:

1. Define objective, model, and hardware plan
2. Prepare and validate training data
3. Run baseline evaluation on fixed eval set
4. Train QLoRA adapter (Unsloth recommended)
5. Merge adapter with base model
6. Export GGUF and package into Ollama
7. Run post-train evaluation and compare reports

The repository now includes scripts for all seven phases.

## Current Project Layout

```
rem-llm/
├── config/
│   ├── config.yaml
│   └── llamafactory_qlora.yaml
├── data/
│   ├── raw.jsonl
│   ├── train.jsonl
│   ├── val.jsonl
│   ├── eval.jsonl
│   ├── sample.jsonl
│   └── dataset_info.json
├── models/                  # ignored in git
├── scripts/
│   ├── prepare_data.py
│   ├── evaluate_model.py
│   ├── compare_reports.py
│   ├── train_unsloth.py
│   ├── train_llamafactory.sh
│   ├── merge_adapter.py
│   ├── export_gguf.sh
│   ├── package_ollama.sh
│   ├── run_pipeline.sh
│   └── train.sh             # old CPU-only Modelfile flow
├── Modelfile                # base prompt-tuned model
├── Modelfile.trained        # for GGUF-trained model packaging
└── requirements.txt
```

## Prerequisites

- Python 3.10+
- Ollama installed and running
- For true QLoRA training: NVIDIA GPU with 8GB+ VRAM (recommended)
- Optional for GGUF conversion: local `llama.cpp` build (`LLAMA_CPP_PATH`)

Install minimal Python requirement:

```bash
python3 -m pip install -r requirements.txt
```

For Unsloth training dependencies:

```bash
pip install unsloth transformers datasets trl accelerate bitsandbytes peft
```

Fallback trainer:

```bash
pip install llamafactory
```

## Quick Start (All 7 Steps at Once)

Run the full orchestrator:

```bash
bash scripts/run_pipeline.sh deepseek-coder:1.3b rem-coder-trained
```

Pipeline outputs:

- baseline report: `models/evals/baseline.json`
- post-train report: `models/evals/post_train.json`
- adapter: `models/rem-coder-lora/`
- merged HF model: `models/rem-coder-merged/`
- gguf: `models/rem-coder-gguf/rem-coder-q4_k_m.gguf`

## Manual Step-by-Step

### 1) Prepare Data

Edit `data/raw.jsonl` with your coding tasks, then:

```bash
python3 scripts/prepare_data.py --config config/config.yaml
```

### 2) Baseline Evaluation

```bash
python3 scripts/evaluate_model.py \
  --config config/config.yaml \
  --model deepseek-coder:1.3b \
  --report models/evals/baseline.json
```

### 3) Train (Unsloth)

```bash
python3 scripts/train_unsloth.py --config config/config.yaml
```

### 4) Fallback Train (LlamaFactory)

```bash
bash scripts/train_llamafactory.sh
```

### 5) Merge Adapter

```bash
python3 scripts/merge_adapter.py --config config/config.yaml
```

### 6) Export GGUF + Package Ollama

```bash
export LLAMA_CPP_PATH=/path/to/llama.cpp
bash scripts/export_gguf.sh
bash scripts/package_ollama.sh rem-coder-trained
```

### 7) Post-Train Evaluation + Compare

```bash
python3 scripts/evaluate_model.py \
  --config config/config.yaml \
  --model rem-coder-trained \
  --report models/evals/post_train.json

python3 scripts/compare_reports.py \
  --baseline models/evals/baseline.json \
  --post models/evals/post_train.json
```

## Notes

- `scripts/train.sh` and `Modelfile` are still useful for CPU-only prompt-tuning.
- Actual learning from your dataset happens in QLoRA (Unsloth or LlamaFactory), not `ollama create` alone.
- Increase dataset size and quality for meaningful coding improvements.

## Evaluation Rubric (Upgraded)

`scripts/evaluate_model.py` now scores each sample with stronger quality signals:

- `non_empty`: model returned a non-empty response
- `has_code`: response appears code-like by token heuristics
- `syntax_ok`: language-aware syntax/shape check
  - Python: parsed using `ast.parse`
  - JavaScript/TypeScript: bracket-balance check
  - SQL: statement-shape check (e.g. `SELECT ... FROM ...`)
- `keyword_overlap`: lexical overlap with reference output
- `quality_score`: weighted composite score per sample

Report-level metrics include:

- `non_empty_rate`
- `has_code_rate`
- `avg_fenced_blocks`
- `avg_keyword_overlap`
- `syntax_ok_rate`
- `avg_quality_score`

`scripts/compare_reports.py` compares all these metrics and also prints per-language quality deltas.
