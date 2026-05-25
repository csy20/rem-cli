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

## Rust CLI (new)

A beginner-focused Rust CLI now lives in `rem-cli/`.

It is designed for:

- basic HTML/CSS coding help
- beginner-safe terminal command guidance
- patch preview workflows with file context

Quick install (recommended):

```bash
curl -fsSL https://raw.githubusercontent.com/csy20/rem-llm/main/install.sh | bash
```

Build and run from source:

```bash
cd rem-cli
cargo build
cargo run -- ask "create a basic html page with linked css"
```

See `rem-cli/README.md` for full usage and safety model.

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
│   ├── benchmark_models.py
│   ├── evaluate_exec.py
│   ├── train_unsloth.py
│   ├── train_llamafactory.sh
│   ├── merge_adapter.py
│   ├── export_gguf.sh
│   ├── package_ollama.sh
│   ├── run_pipeline.sh
│   ├── write_run_metadata.py
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

Fast iteration mode (skip dependency install and cached baseline eval):

```bash
SKIP_DEPS=1 SKIP_BASELINE_IF_EXISTS=1 bash scripts/run_pipeline.sh deepseek-coder:1.3b rem-coder-trained
```

Pipeline outputs:

- baseline report: `models/evals/baseline.json`
- baseline executable report: `models/evals/baseline_exec.json`
- post-train report: `models/evals/post_train.json`
- post-train executable report: `models/evals/post_train_exec.json`
- adapter: `models/rem-coder-lora/`
- merged HF model: `models/rem-coder-merged/`
- gguf: `models/rem-coder-gguf/rem-coder-q4_k_m.gguf`
- run metadata: `models/experiments/<run-id>/metadata.json`

## Manual Step-by-Step

### 1) Prepare Data

Edit `data/raw.jsonl` with your coding tasks, then:

```bash
python3 scripts/prepare_data.py --config config/config.yaml
```

The data prep step now uses fingerprint caching and skips work when `data/raw.jsonl`
and split settings are unchanged.

Force regeneration:

```bash
python3 scripts/prepare_data.py --config config/config.yaml --force
```

Generate beginner web + terminal synthetic dataset:

```bash
python3 -m remllm.cli data generate \
  --domain beginner \
  --output data/domains/beginner/raw.generated.jsonl
```

### 2) Baseline Evaluation

```bash
python3 scripts/evaluate_model.py \
  --config config/config.yaml \
  --model deepseek-coder:1.3b \
  --report models/evals/baseline.json

python3 scripts/evaluate_exec.py \
  --config config/config.yaml \
  --model deepseek-coder:1.3b \
  --report models/evals/baseline_exec.json

python3 -m remllm.cli eval beginner \
  --config config/domains/beginner_web_cli.yaml \
  --model deepseek-coder:1.3b \
  --report models/evals/beginner_baseline.json
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

Export multiple quantizations in one pass:

```bash
export LLAMA_CPP_PATH=/path/to/llama.cpp
QUANT_LIST="q4_k_m q5_k_m q8_0" bash scripts/export_gguf.sh
```

Package a specific quant:

```bash
bash scripts/package_ollama.sh rem-coder-trained-q5 q5_k_m
```

### 7) Post-Train Evaluation + Compare

```bash
python3 scripts/evaluate_model.py \
  --config config/config.yaml \
  --model rem-coder-trained \
  --report models/evals/post_train.json

python3 scripts/evaluate_exec.py \
  --config config/config.yaml \
  --model rem-coder-trained \
  --report models/evals/post_train_exec.json

python3 scripts/compare_reports.py \
  --baseline models/evals/baseline.json \
  --post models/evals/post_train.json \
  --baseline-exec models/evals/baseline_exec.json \
  --post-exec models/evals/post_train_exec.json
```

## Experiment Metadata

`scripts/run_pipeline.sh` now writes run metadata for reproducible comparisons:

```bash
models/experiments/<run-id>/metadata.json
```

Set a custom run id:

```bash
RUN_ID=exp-20260518-01 bash scripts/run_pipeline.sh deepseek-coder:1.3b rem-coder-trained
```

## Benchmark Model Variants

Benchmark multiple Ollama models on shared prompts for latency and throughput:

```bash
python3 scripts/benchmark_models.py \
  --models rem-coder-trained-q4,rem-coder-trained-q5,rem-coder-trained-q8 \
  --eval-file data/eval.jsonl \
  --max-samples 20 \
  --report models/evals/benchmark.json
```

## Notes

- `scripts/train.sh` and `Modelfile` are still useful for CPU-only prompt-tuning.
- Actual learning from your dataset happens in QLoRA (Unsloth or LlamaFactory), not `ollama create` alone.
- Increase dataset size and quality for meaningful coding improvements.
- `evaluate_exec.py` supports executable checks for Python, JavaScript (Node syntax check), and SQL (SQLite execution shape).

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
