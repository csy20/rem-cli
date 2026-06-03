"""Teacher-distilled SFT data with HTTP-API sampling.

The v1 `distill_ollama` uses the `ollama run` CLI which doesn't support
temperature or multi-sample. v2 hits the /api/generate endpoint so we can
control sampling. Used to expand the curated set with higher-quality,
temperature-controlled completions from a teacher.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path

from remllm.data.loader import load_jsonl, write_jsonl
from remllm.data.ollama_client import GenerationParams, ollama_sample_n
from remllm.logging import get_logger


SYSTEM_PROMPT = (
    "You are a precise coding assistant. Provide a complete, idiomatic answer. "
    "If the question is about code, include a properly fenced code block."
)


@dataclass
class DistillConfig:
    input_path: Path
    output_path: Path
    teacher_model: str = "qwen2.5-coder:7b"
    student_model: str = "qwen2.5-coder:1.5b"
    n_samples: int = 1
    temperature: float = 0.7
    max_samples: int = 0
    seed: int = 42
    timeout_s: int = 120
    base_url: str = "http://localhost:11434"
    num_predict: int = 1024


def distill_dataset_v2(cfg: DistillConfig) -> dict:
    log = get_logger(operation="distill_v2", teacher=cfg.teacher_model)
    rows = load_jsonl(cfg.input_path)
    if cfg.max_samples > 0:
        rows = rows[: cfg.max_samples]
    log.info("distill_input", count=len(rows))

    distilled: list[dict] = []
    failed = 0
    for i, row in enumerate(rows):
        instr = row.get("instruction", "")
        user_input = row.get("input", "")
        prompt = f"{instr}\n\nContext:\n{user_input}" if user_input else instr
        params = GenerationParams(
            model=cfg.teacher_model,
            prompt=prompt,
            temperature=cfg.temperature,
            seed=cfg.seed + i,
            num_predict=cfg.num_predict,
            system=SYSTEM_PROMPT,
        )
        try:
            samples = ollama_sample_n(
                params,
                n=cfg.n_samples,
                base_url=cfg.base_url,
                timeout_s=cfg.timeout_s,
            )
        except Exception as exc:
            log.warning("distill_sample_failed", idx=i, error=str(exc))
            failed += 1
            continue

        for j, sample in enumerate(samples):
            if not sample.strip():
                continue
            distilled.append(
                {
                    "instruction": instr,
                    "input": user_input,
                    "output": sample,
                    "domain": row.get("domain", "general"),
                    "difficulty": row.get("difficulty", "intermediate"),
                    "tags": row.get("tags", []) + ["distilled"],
                    "source": f"distill:{cfg.teacher_model}:{i}:{j}",
                }
            )

        if (i + 1) % 25 == 0:
            log.info("distill_progress", processed=i + 1, written=len(distilled))

    write_jsonl(cfg.output_path, distilled)
    stats = {
        "input_rows": len(rows),
        "distilled_written": len(distilled),
        "failed_samples": failed,
        "teacher_model": cfg.teacher_model,
        "n_samples": cfg.n_samples,
        "temperature": cfg.temperature,
        "output": str(cfg.output_path),
    }
    log.info("distill_complete", **stats)
    print(json.dumps(stats, indent=2))
    return stats
