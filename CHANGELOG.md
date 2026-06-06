# Changelog

All notable changes to this project are documented here. Versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Removed `rem-cli/target/` (~5,942 files, ~2.4 GB of build artifacts)
  from the git index. The directory is already listed in `.gitignore`
  but had been committed before the ignore rule landed. The on-disk
  `target/` is preserved; subsequent `cargo build` is incremental. Repo
  size on `origin/main` shrinks accordingly. `.gitignore` additionally
  ignores `**/*.rs.bk` and `Cargo.lock.bak`.

### Added

- `/find <query>` — in-project text search. Walks the project (skipping
  `node_modules`, `target`, `.git`, `dist`, `build`, `.rem`, lock files,
  and common binary suffixes), returns matching lines with one-based
  `path:line:column`. Caps at 500 matches / 8 KiB per file. Pure local,
  no LLM, no network. New module `rem-cli/src/find.rs` + integration
  test `rem-cli/tests/find.rs`.

### Removed

- **Python training pipeline** — the entire QLoRA fine-tuning, data
  curation, GGUF export, and eval harness that previously lived at the
  repo root has been removed. The repo is now exclusively the `rem`
  Rust CLI (`rem-cli/`). The CLI is fully self-contained: it talks to
  Ollama and does not need any Python tooling to run.

  Specifically deleted:
  - `src/remllm/` (Python package: data, train, eval, context, export, cli)
  - `scripts/` (prepare_data, train_*, merge_adapter, export_gguf,
    package_ollama*, fetch_benchmarks, run_full_eval, run_pipeline, etc.)
  - `tests/` (181 pytest cases)
  - `data/` (raw, train, val, eval, sources, curated, preferences, domains)
  - `models/` (curriculum, evals, experiments, codebase_index.json)
  - `config/` (config.yaml, llamafactory_qlora.yaml, domains/)
  - `Modelfile`, `Modelfile.trained`
  - `pyproject.toml`, `requirements.txt`

### Changed

- `README.md` rewritten to focus on `rem` CLI install / usage only.
- `CHANGELOG.md` older entries (v0.3.0 / v0.2.0) preserved below for
  history; they describe the removed Python pipeline.

## [0.3.0] — Scaling Week (REMOVED)

A full data + training + eval pipeline that scaled `rem-coder` from a
1.5B baseline to a v0.3.0 candidate. This release has been removed
from the repo; kept here for historical reference only.

### Added

- **Day 1 — Data foundation**
  - `data/curator.py` — end-to-end curation pipeline (sources → normalize →
    exact-dedup → MinHash near-dedup → heuristic filter → mix → split)
  - `data/dedup.py` — LSH-banded MinHash for O(n) near-dedup at any scale
  - `data/filter.py` — local heuristic filter (no LLM round-trip) plus the
    original Ollama-based filter
  - `data/curate` CLI command + 4 registered sources (3 HF datasets + local)
  - `data/fetch` CLI command for ad-hoc HuggingFace dataset pulls
  - `data/sources list` for the source registry
  - 4,914 CodeAlpaca + 8,000 Magicoder + 7,999 Evol-Code rows pulled
  - **6,430 train / 357 val / 200 eval** rows at `data/curated/v1/`

- **Day 2 — Teacher distillation + DPO preference data**
  - `data/ollama_client.py` — HTTP API client supporting `temperature`,
    `top_p`, `seed`, multi-sample
  - `data/dpo_generator.py` — sampling-based (prompt, chosen, rejected)
    builder with executable judge
  - `data/distill_v2.py` — teacher distillation with temperature sampling
  - `data/dpo` and `data/distill-v2` CLI commands
  - Preference data at `data/preferences/v1/`

- **Day 3 — Curriculum SFT**
  - `data/difficulty.py` — AST + vocab + code-density scorer with adaptive
    percentile bands
  - `data/score-difficulty` CLI command
  - `train/unsloth.py` — added `split_curriculum_stages()` (3-stage split
    with cumulative inclusion)
  - `train curriculum` CLI command
  - Stage 1 (easy): 2,140 rows. Stage 2 (+intermediate): 4,286. Stage 3 (all):
    6,430. Manifest at `models/curriculum/v1/manifest.json`.

- **Day 4 — DPO + 8K RoPE scaling**
  - `train dpo-v2` CLI command wiring `train/dpo.py` to Day-2 preference data
  - `eval/long_context_probe.py` — needle-in-haystack behavioral probe for
    RoPE-scaled context
  - `eval long-context` CLI command
  - `config/config.yaml` now sets `rope_scaling: true, factor: 2.0` and
    `curriculum: true` by default
  - `Modelfile.trained` updated to 8K context (`num_ctx 8192`)

- **Day 5 — Eval suite + benchmarks + multi-quant packaging**
  - `scripts/fetch_benchmarks.py` — downloads HumanEval (164) + MBPP (427)
  - `scripts/package_ollama_multi.py` — q4_k_m / q5_k_m / q8_0 sweep into
    separate Ollama models
  - `scripts/run_full_eval.py` — one-shot runner for quality + exec + HumanEval
    + MBPP + long-context probe + latency
  - Baseline eval report at `models/evals/full_baseline_v030.json`

- **Day 6 — Reproduce, regress, document**
  - 43 new regression tests in `tests/test_data_pipeline_v030.py` and
    `tests/test_dpo_longctx_bench_v030.py`
  - **181 total tests pass**
  - v0.3.0 section in `README.md` with pipeline commands
  - This CHANGELOG entry

### Fixed

- `data/mixer.py` — `len(pool[name])` reference error when `total_ratio == 0`
- `data/dedup.py` — quadratic O(n²) near-dedup replaced with LSH-banded
  MinHash; 20k rows process in ~20s

### Known limitations (Variant A — no cloud)

- Training itself still requires a GPU; the no-GPU baseline can run all
  data and eval infrastructure end-to-end but cannot actually train.
- HumanEval pass@1 on a 3-sample smoke is 0% for the existing `rem-coder`
  baseline because the harness prompt asks for `function body` only and
  the existing model returns the whole function. Full eval on a 164-task
  pass is the next-step measurement.

## [0.2.0] — Earlier releases (REMOVED)

See git history for the pre-pipeline state.
