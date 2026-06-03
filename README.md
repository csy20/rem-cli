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
- **Three interactive modes**: CHAT (conversation), CODE (generation), PLAN (analysis)
- **30+ slash commands**: `/help`, `/mode`, `/plan`, `/clear`, `/reset`, `/explain`, `/test`, `/refactor`, `/write`, `/save`, `/dir`, `/search`, `/code`, `/files`, `/undo`, `/diff`, `/tokens`, `/config`, `/why`, `/init`, `/memory`, `/compact`, `/goal`, `/copy`, `/lint`, `/review`, `/resume`
- **`@` references**: `fix @src/main.rs` injects file/directory context inline
- **Persistent memory**: `.rem/memory.md` with auto-generation via `/init`
- **Pipe mode**: `git diff | rem` for non-interactive analysis
- **Autonomous loop**: `/goal <condition>` keeps working until done

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
│   ├── dataset_info.json
│   ├── curated/v1/                # Day-1 deliverable: 6.4k curated rows
│   ├── preferences/v1/            # Day-2 deliverable: DPO pairs
│   ├── sources/                   # cached HF dataset pulls
│   └── benchmarks/                # Day-5: HumanEval + MBPP
├── models/                  # ignored in git
│   ├── curriculum/v1/             # Day-3: 3-stage curriculum splits
│   └── evals/                     # baseline + v0.3.0 eval reports
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
│   ├── package_ollama_multi.py    # Day-5: q4/q5/q8 multi-quant packager
│   ├── fetch_benchmarks.py        # Day-5: HumanEval + MBPP fetcher
│   ├── run_full_eval.py           # Day-5: one-shot full eval suite
│   ├── run_pipeline.sh
│   ├── write_run_metadata.py
│   └── train.sh                   # old CPU-only Modelfile flow
├── src/remllm/
│   ├── data/
│   │   ├── curator.py             # Day-1: end-to-end curation
│   │   ├── dedup.py               # LSH MinHash for O(n) near-dedup
│   │   ├── filter.py              # heuristic + Ollama filter tiers
│   │   ├── difficulty.py          # Day-3: AST+vocab+code-density scorer
│   │   ├── dpo_generator.py       # Day-2: sampling-based pair builder
│   │   ├── distill_v2.py          # Day-2: teacher distillation
│   │   ├── ollama_client.py       # HTTP API for sampling/temperature
│   │   └── ...
│   ├── train/unsloth.py           # Day-3: staged curriculum SFT
│   ├── train/dpo.py               # Day-4: DPO trainer (refactored)
│   └── eval/long_context_probe.py # Day-4: RoPE-scaling verification
├── Modelfile                # base prompt-tuned model
├── Modelfile.trained        # Day-4: 8K context, RoPE-scaled
├── tests/                   # 181 tests, 100% pass
└── requirements.txt
```

## v0.3.0 — Scaling Week Summary

This release adds a full data + training + eval pipeline to scale `rem-coder`
from a 1.5B baseline to a v0.3.0 candidate that can be trained and evaluated
end-to-end on consumer hardware.

| Day | Deliverable | Status |
|---|---|---|
| 1 | `data/curated/v1/` — 6,430 train / 357 val / 200 eval from 20,605 raw | ✅ |
| 2 | `data/preferences/v1/dpo.jsonl` — DPO pairs via executable judge | ✅ |
| 3 | `models/curriculum/v1/` — 3-stage curriculum splits + difficulty scorer | ✅ |
| 4 | DPO trainer + 8K RoPE scaling + long-context probe | ✅ |
| 5 | HumanEval + MBPP + 3-quant GGUF packager + full eval runner | ✅ |
| 6 | 181 regression tests, baseline report, README, CHANGELOG, tag | ✅ |

### Baseline numbers (`rem-coder:latest`, 3-sample eval, no cloud)

| Metric | Value | Notes |
|---|---|---|
| HumanEval pass@1 | 10% (1/10) | full 164-task run pending GPU |
| HumanEval pass@1 (3-sample smoke) | 0% (0/3) | prompt was `return ONLY body`, not full function |
| MBPP pass@1 (3-sample smoke) | 0% (0/3) | small sample |
| Long-context recall | 1,024 / 2,048 / 4,096 / 8,192 → 1K HIT only | RoPE-scaled 8K target |
| Latency | ~5 tok/s on CPU | 1.5B Qwen on consumer hardware |

### Reproducing the v0.3.0 pipeline (CPU-only path)

```bash
# Day 1 — data foundation
python3 -m remllm.cli data fetch sahil2801/CodeAlpaca-20k \\
    --output data/sources/codealpaca.jsonl --max-samples 5000
python3 -m remllm.cli data fetch ise-uiuc/Magicoder-OSS-Instruct-75K \\
    --output data/sources/magicoder.jsonl --max-samples 8000
python3 -m remllm.cli data fetch nickrosh/Evol-Instruct-Code-80k-v1 \\
    --output data/sources/evol_code.jsonl --max-samples 8000
python3 -m remllm.cli data curate \\
    --sources local_synthetic,hf_codealpaca,hf_magicoder,hf_evol_code \\
    --output-dir data/curated/v1 --target-size 15000

# Day 2 — DPO pair generation (requires Ollama)
python3 -m remllm.cli data dpo \\
    --input data/curated/v1/train.jsonl \\
    --output data/preferences/v1/dpo.jsonl \\
    --model rem-coder:latest --max-prompts 500 --n-samples 3

# Day 3 — difficulty + 3-stage curriculum
python3 -m remllm.cli data score-difficulty \\
    --input data/curated/v1/train.jsonl \\
    --output data/curated/v1/train.scored.jsonl
python3 -m remllm.cli train curriculum \\
    --input data/curated/v1/train.scored.jsonl \\
    --output-dir models/curriculum/v1

# Day 4 — RoPE scaling probe
python3 -m remllm.cli eval long-context \\
    --model rem-coder:latest \\
    --targets 1024,2048,4096,6000,8000

# Day 5 — full eval + benchmarks
python3 scripts/fetch_benchmarks.py
python3 scripts/run_full_eval.py --model rem-coder:latest \\
    --max-eval 20 --max-humaneval 164 --max-mbpp 257
```

### After training (Day 4+) — Variant A on GPU

```bash
# 3-stage SFT (run on a GPU box with unsloth installed)
for s in 1 2 3; do
    STAGE=$(ls models/curriculum/v1/stage_${s}_*.jsonl | head -1)
    python3 -m remllm.train.unsloth --config config/config.yaml  # then load $STAGE
done

# DPO on preference data
python3 -m remllm.cli train dpo-v2 \\
    --dpo-file data/preferences/v1/dpo.jsonl \\
    --epochs 1 --beta 0.1 --learning-rate 5e-6

# Merge, GGUF, package q4/q5/q8
python3 -m remllm.export.gguf --quants q4_k_m,q5_k_m,q8_0
python3 scripts/package_ollama_multi.py \\
    --base-name rem-coder-v2 --quants q4_k_m,q5_k_m,q8_0
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
bash scripts/run_pipeline.sh qwen2.5-coder:1.5b rem-coder-trained
```

Fast iteration mode (skip dependency install and cached baseline eval):

```bash
SKIP_DEPS=1 SKIP_BASELINE_IF_EXISTS=1 bash scripts/run_pipeline.sh qwen2.5-coder:1.5b rem-coder-trained
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

This generates 7 training examples across HTML, CSS, and terminal domains. Available domains include: `beginner`, `nextjs`, `backend`, `devops`, `mobile`, `analysis`, and language-specific domains (python, rust, go, etc.).

After generation, prepare the dataset for training:

```bash
python3 -m remllm.cli data prepare --config config/config.yaml
```

Or target a specific domain's config:

```bash
python3 -m remllm.cli data prepare --config config/domains/beginner_web_cli.yaml
```

### 2) Baseline Evaluation

```bash
python3 scripts/evaluate_model.py \
  --config config/config.yaml \
  --model qwen2.5-coder:1.5b \
  --report models/evals/baseline.json

python3 scripts/evaluate_exec.py \
  --config config/config.yaml \
  --model qwen2.5-coder:1.5b \
  --report models/evals/baseline_exec.json

python3 -m remllm.cli eval beginner \
  --config config/domains/beginner_web_cli.yaml \
  --model qwen2.5-coder:1.5b \
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
RUN_ID=exp-20260518-01 bash scripts/run_pipeline.sh qwen2.5-coder:1.5b rem-coder-trained
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
