#!/usr/bin/env python3

import argparse
import json
import statistics
import subprocess
import time
from pathlib import Path


def load_jsonl(path: Path) -> list[dict]:
    rows = []
    with path.open("r", encoding="utf-8") as handle:
        for raw in handle:
            raw = raw.strip()
            if raw:
                rows.append(json.loads(raw))
    return rows


def build_prompt(row: dict) -> str:
    prompt = str(row.get("instruction", "")).strip()
    context = str(row.get("input", "")).strip()
    if context:
        prompt = f"{prompt}\n\nContext:\n{context}"
    return prompt


def run_prompt(model_name: str, prompt: str, timeout_s: int) -> tuple[str, float]:
    start = time.perf_counter()
    process = subprocess.run(
        ["ollama", "run", model_name, prompt],
        capture_output=True,
        text=True,
        check=False,
        timeout=timeout_s,
    )
    elapsed = time.perf_counter() - start
    if process.returncode != 0:
        raise RuntimeError(process.stderr.strip() or "ollama run failed")
    return process.stdout.strip(), elapsed


def summarize(times: list[float], output_lengths: list[int]) -> dict:
    if not times:
        return {
            "count": 0,
            "avg_latency_s": 0.0,
            "p50_latency_s": 0.0,
            "p95_latency_s": 0.0,
            "avg_chars_per_second": 0.0,
        }
    ordered = sorted(times)
    p50_index = int(0.5 * (len(ordered) - 1))
    p95_index = int(0.95 * (len(ordered) - 1))
    throughput_samples = []
    for idx, elapsed in enumerate(times):
        if elapsed > 0:
            throughput_samples.append(output_lengths[idx] / elapsed)
    avg_throughput = statistics.mean(throughput_samples) if throughput_samples else 0.0
    return {
        "count": len(times),
        "avg_latency_s": round(statistics.mean(times), 4),
        "p50_latency_s": round(ordered[p50_index], 4),
        "p95_latency_s": round(ordered[p95_index], 4),
        "avg_chars_per_second": round(avg_throughput, 2),
    }


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Benchmark Ollama model latency/throughput."
    )
    parser.add_argument(
        "--models",
        required=True,
        help="Comma-separated Ollama model names.",
    )
    parser.add_argument(
        "--eval-file",
        default="data/eval.jsonl",
        help="JSONL file with instruction/input fields.",
    )
    parser.add_argument(
        "--max-samples",
        type=int,
        default=20,
        help="Maximum number of eval samples per model.",
    )
    parser.add_argument(
        "--timeout-s",
        type=int,
        default=180,
        help="Per-request timeout in seconds.",
    )
    parser.add_argument(
        "--report",
        default="models/evals/benchmark.json",
        help="Output benchmark report path.",
    )
    args = parser.parse_args()

    eval_path = Path(args.eval_file)
    rows = load_jsonl(eval_path)
    if not rows:
        raise ValueError(f"No rows found in {eval_path}")

    sample_rows = rows[: max(1, min(args.max_samples, len(rows)))]
    model_names = [name.strip() for name in args.models.split(",") if name.strip()]
    if not model_names:
        raise ValueError("No model names provided")

    report = {
        "eval_file": str(eval_path),
        "num_samples": len(sample_rows),
        "models": {},
    }

    for model_name in model_names:
        latencies = []
        output_lengths = []
        failures = []
        for index, row in enumerate(sample_rows, start=1):
            prompt = build_prompt(row)
            try:
                output, elapsed = run_prompt(model_name, prompt, args.timeout_s)
                latencies.append(elapsed)
                output_lengths.append(len(output))
            except Exception as exc:
                failures.append({"sample_index": index, "error": str(exc)})
        report["models"][model_name] = {
            "summary": summarize(latencies, output_lengths),
            "failures": failures,
        }

    report_path = Path(args.report)
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(f"Wrote benchmark report: {report_path}")
    print(json.dumps(report["models"], indent=2))


if __name__ == "__main__":
    main()
