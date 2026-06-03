"""DPO preference-pair generator.

For each prompt in the curated training set:
1. Sample N candidate completions from a generator model at temperature 0.8.
2. Score each with the existing `quality.score_response` heuristics.
3. For code prompts, additionally run the executable judge (`executable.evaluate_row`).
4. Pick the highest-scoring candidate as `chosen` and the lowest as `rejected`.

Outputs JSONL with: {prompt, chosen, rejected, chosen_score, rejected_score,
chosen_exec, rejected_exec, source, language, ...}.

The output is consumable by `remllm train dpo` (Day 4) which already
expects {prompt, chosen, rejected} schema (see train/dpo.py:_format_dpo_row).
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from pathlib import Path
from typing import Iterator

from remllm.data.loader import load_jsonl, write_jsonl
from remllm.data.ollama_client import GenerationParams, ollama_sample_n
from remllm.eval.executable import check_python_exec, check_javascript_exec
from remllm.eval.quality import extract_code, score_response
from remllm.logging import get_logger


CODE_KEYWORDS = (
    "code",
    "function",
    "implement",
    "write",
    "create",
    "fix",
    "refactor",
    "class",
    "method",
    "script",
    "program",
    "algorithm",
    "bug",
    "compile",
    "syntax",
    "regex",
    "sql",
    "select",
    "from",
    "where",
)


def _is_code_prompt(row: dict) -> bool:
    text = (row.get("instruction", "") + " " + row.get("input", "")).lower()
    return any(kw in text for kw in CODE_KEYWORDS)


def _build_prompt(row: dict) -> str:
    instr = row.get("instruction", "")
    user_input = row.get("input", "")
    if user_input:
        return f"{instr}\n\nContext:\n{user_input}"
    return instr


@dataclass
class DPOPairConfig:
    input_path: Path
    output_path: Path
    generator_model: str = "qwen2.5-coder:1.5b"
    n_samples: int = 4
    temperature: float = 0.8
    max_prompts: int = 10000
    seed: int = 42
    min_score_gap: float = 0.1
    timeout_s: int = 60
    base_url: str = "http://localhost:11434"
    num_predict: int = 512
    skip_non_code: bool = False
    require_exec_judge: bool = False


def _score_candidate(
    row: dict, response: str, language_hint: str | None = None
) -> dict:
    """Score a single response: heuristic quality + executable check."""
    metrics = score_response(row, response)
    exec_result: dict = {
        "executable_checked": 0,
        "executable_ok": 0,
        "detail": "skipped",
    }
    language = metrics.get("language", "unknown")
    if language == "python":
        code = extract_code(response)
        ok, detail = check_python_exec(code, timeout_s=15)
        exec_result = {
            "executable_checked": 1,
            "executable_ok": int(ok),
            "detail": detail,
        }
    elif language == "javascript":
        code = extract_code(response)
        js_ok, detail = check_javascript_exec(code, timeout_s=15)
        if js_ok is None:
            exec_result = {
                "executable_checked": 0,
                "executable_ok": 0,
                "detail": detail,
            }
        else:
            exec_result = {
                "executable_checked": 1,
                "executable_ok": int(js_ok),
                "detail": detail,
            }
    bonus = 0.4 if exec_result.get("executable_ok") else 0.0
    metrics["composite"] = round(metrics.get("quality_score", 0.0) + bonus, 4)
    metrics["executable"] = exec_result
    return metrics


def generate_dpo_pairs(cfg: DPOPairConfig) -> dict:
    log = get_logger(operation="dpo_generate", model=cfg.generator_model)
    rows = load_jsonl(cfg.input_path)
    if cfg.max_prompts > 0:
        rows = rows[: cfg.max_prompts]
    log.info("dpo_prompts_loaded", count=len(rows))

    pairs: list[dict] = []
    skipped_no_gap = 0
    skipped_no_pair = 0
    failed_samples = 0

    for i, row in enumerate(rows):
        if cfg.skip_non_code and not _is_code_prompt(row):
            continue
        prompt = _build_prompt(row)
        params = GenerationParams(
            model=cfg.generator_model,
            prompt=prompt,
            temperature=cfg.temperature,
            seed=cfg.seed + i,
            num_predict=cfg.num_predict,
        )
        try:
            samples = ollama_sample_n(
                params, n=cfg.n_samples, base_url=cfg.base_url, timeout_s=cfg.timeout_s
            )
        except Exception as exc:
            log.warning("sample_failed", idx=i, error=str(exc))
            failed_samples += 1
            continue

        if len(samples) < 2:
            skipped_no_pair += 1
            continue

        scored: list[tuple[float, str, dict]] = []
        for s in samples:
            metrics = _score_candidate(row, s)
            scored.append((metrics["composite"], s, metrics))
        scored.sort(key=lambda x: x[0], reverse=True)

        best_score, best_text, best_metrics = scored[0]
        worst_score, worst_text, worst_metrics = scored[-1]

        if best_score - worst_score < cfg.min_score_gap:
            skipped_no_gap += 1
            continue

        if cfg.require_exec_judge:
            if not best_metrics["executable"].get("executable_ok"):
                continue
            if worst_metrics["executable"].get("executable_ok"):
                continue

        pair = {
            "prompt": prompt,
            "chosen": best_text,
            "rejected": worst_text,
            "chosen_score": best_score,
            "rejected_score": worst_score,
            "chosen_metrics": best_metrics,
            "rejected_metrics": worst_metrics,
            "source": row.get("source") or row.get("domain") or "curated",
            "language": best_metrics.get("language", "unknown"),
            "difficulty": row.get("difficulty", "intermediate"),
        }
        pairs.append(pair)

        if (i + 1) % 50 == 0:
            log.info(
                "dpo_progress",
                processed=i + 1,
                pairs=len(pairs),
                skipped_gap=skipped_no_gap,
                skipped_pair=skipped_no_pair,
            )

    write_jsonl(cfg.output_path, pairs)

    stats = {
        "prompts_processed": len(rows),
        "pairs_written": len(pairs),
        "skipped_no_score_gap": skipped_no_gap,
        "skipped_too_few_samples": skipped_no_pair,
        "failed_samples": failed_samples,
        "generator_model": cfg.generator_model,
        "n_samples_per_prompt": cfg.n_samples,
        "temperature": cfg.temperature,
        "min_score_gap": cfg.min_score_gap,
        "output": str(cfg.output_path),
    }
    log.info("dpo_generation_complete", **stats)
    print(json.dumps(stats, indent=2))
    return stats
