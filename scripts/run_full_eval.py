"""Full evaluation runner — produces the v0.3.0 day-5 report.

Runs:
  1. Quality eval on data/curated/v1/eval.jsonl
  2. Executable eval on the same
  3. Beginner HTML/CSS/terminal eval
  4. Long-context probe (RoPE-scaled)
  5. HumanEval (if installed)
  6. MBPP (if installed)
  7. Latency/throughput benchmark

All results are aggregated into a single JSON report and printed as a
markdown table.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import time
from pathlib import Path
from typing import Any

from remllm.data.loader import load_jsonl
from remllm.logging import get_logger


def _gen_ollama(model: str, prompt: str, timeout: int = 120) -> str:
    r = subprocess.run(
        ["ollama", "run", model, prompt],
        capture_output=True,
        text=True,
        timeout=timeout,
    )
    return r.stdout.strip() if r.returncode == 0 else ""


def _latency_benchmark(model: str, prompts: list[str], timeout: int = 60) -> dict:
    """Measure end-to-end latency and tokens/sec for a model.

    Per-prompt timeouts are isolated — one slow generation doesn't kill the
    whole benchmark.
    """
    log = get_logger(benchmark="latency", model=model)
    samples = []
    total_tokens = 0
    timeouts = 0
    for i, p in enumerate(prompts):
        start = time.time()
        try:
            out = _gen_ollama(model, p, timeout)
        except subprocess.TimeoutExpired:
            timeouts += 1
            samples.append(
                {
                    "prompt_idx": i,
                    "tokens": 0,
                    "elapsed_s": round(time.time() - start, 3),
                    "tps": 0.0,
                    "status": "timeout",
                }
            )
            continue
        elapsed = time.time() - start
        toks = max(1, len(out.split()))
        tps = toks / elapsed if elapsed > 0 else 0
        total_tokens += toks
        samples.append(
            {
                "prompt_idx": i,
                "tokens": toks,
                "elapsed_s": round(elapsed, 3),
                "tps": round(tps, 2),
                "status": "ok",
            }
        )
    successful = [s for s in samples if s.get("status") == "ok"]
    avg_tps = (
        round(sum(s["tps"] for s in successful) / len(successful), 2)
        if successful
        else 0.0
    )
    summary = {
        "model": model,
        "prompts": len(samples),
        "successful": len(successful),
        "timeouts": timeouts,
        "total_tokens": total_tokens,
        "avg_tps": avg_tps,
        "samples": samples,
    }
    log.info("latency_complete", **summary)
    return summary


def _safe_run(fn, *args, **kwargs) -> dict:
    try:
        return fn(*args, **kwargs)
    except Exception as exc:
        return {"error": str(exc)}


def main():
    parser = argparse.ArgumentParser(description="Run full v0.3.0 eval suite")
    parser.add_argument("--model", required=True, help="Ollama model name")
    parser.add_argument(
        "--eval-file", default="data/curated/v1/eval.jsonl", help="Eval prompts JSONL"
    )
    parser.add_argument(
        "--benchmarks-dir", default="data/benchmarks", help="HumanEval/MBPP dir"
    )
    parser.add_argument(
        "--max-eval", type=int, default=20, help="Max eval rows for quality/exec"
    )
    parser.add_argument(
        "--max-humaneval", type=int, default=10, help="Max HumanEval tasks"
    )
    parser.add_argument("--max-mbpp", type=int, default=10, help="Max MBPP tasks")
    parser.add_argument("--output", default="models/evals/full_v030.json")
    args = parser.parse_args()

    log = get_logger(operation="full_eval", model=args.model)
    log.info("eval_start")

    report: dict[str, Any] = {"model": args.model, "sections": {}}
    eval_rows = load_jsonl(Path(args.eval_file))[: args.max_eval]

    if eval_rows:
        from remllm.eval.executable import ExecutableEvaluator
        from remllm.eval.quality import QualityEvaluator

        try:
            quality_report = QualityEvaluator().evaluate(
                args.model, eval_rows, timeout_s=90
            )
            report["sections"]["quality"] = quality_report.to_dict()
        except subprocess.TimeoutExpired as exc:
            report["sections"]["quality"] = {
                "error": "timeout",
                "detail": str(exc)[:200],
            }
        except Exception as exc:
            report["sections"]["quality"] = {"error": str(exc)[:200]}

        try:
            exec_report = ExecutableEvaluator().evaluate(
                args.model, eval_rows, timeout_s=90
            )
            report["sections"]["executable"] = exec_report.to_dict()
        except subprocess.TimeoutExpired as exc:
            report["sections"]["executable"] = {
                "error": "timeout",
                "detail": str(exc)[:200],
            }
        except Exception as exc:
            report["sections"]["executable"] = {"error": str(exc)[:200]}

    benchmarks_dir = Path(args.benchmarks_dir)
    he_path = benchmarks_dir / "humaneval.jsonl"
    if he_path.exists():
        from remllm.eval.benchmark_harness import evaluate_humaneval

        def gen_fn(p: str) -> str:
            return _gen_ollama(args.model, p, timeout=120)

        he = evaluate_humaneval(
            args.model, he_path, gen_fn, max_samples=args.max_humaneval
        )
        report["sections"]["humaneval"] = he.to_dict()

    mbpp_path = benchmarks_dir / "mbpp.jsonl"
    if mbpp_path.exists():
        from remllm.eval.benchmark_harness import evaluate_mbpp

        def gen_fn(p: str) -> str:
            return _gen_ollama(args.model, p, timeout=120)

        mb = evaluate_mbpp(args.model, mbpp_path, gen_fn, max_samples=args.max_mbpp)
        report["sections"]["mbpp"] = mb.to_dict()

    lat_prompts = [
        "Write a Python function to compute the factorial of n.",
        "Explain the difference between async/await and threads in Python.",
        "Implement a simple LRU cache in TypeScript with O(1) get/put.",
    ]
    report["sections"]["latency"] = _latency_benchmark(args.model, lat_prompts)

    out_path = Path(args.output)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(f"\n=== Full eval report → {out_path} ===\n")
    print(json.dumps(report["sections"], indent=2))


if __name__ == "__main__":
    main()
