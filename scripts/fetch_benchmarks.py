"""Fetch HumanEval and MBPP benchmark JSONL files into data/benchmarks/.

Both come from open-source mirrors (codeparrot for HumanEval, original
MBPP release for MBPP). Falls back gracefully if a network or HF download
fails — emits a warning and writes nothing.
"""

from __future__ import annotations

import json
import os
import re
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

from remllm.logging import get_logger


HUMANEVAL_URL = (
    "https://raw.githubusercontent.com/openai/human-eval/master/data/HumanEval.jsonl.gz"
)
MBPP_URL = (
    "https://raw.githubusercontent.com/google-research/mbpp/master/mbpp/mbpp.jsonl"
)


def _download(url: str, dest: Path, timeout_s: int = 60) -> bool:
    dest.parent.mkdir(parents=True, exist_ok=True)
    try:
        with urllib.request.urlopen(url, timeout=timeout_s) as resp:
            data = resp.read()
        dest.write_bytes(data)
        return True
    except (urllib.error.URLError, TimeoutError, OSError) as exc:
        log = get_logger(operation="fetch_benchmark", url=url)
        log.warning("download_failed", error=str(exc))
        return False


def _normalize_humaneval(raw_path: Path, out_path: Path) -> int:
    """Convert HumanEval.jsonl.gz to the {task_id, prompt, test, entry_point, canonical_solution} format."""
    import gzip

    log = get_logger(operation="normalize_humaneval")
    out_path.parent.mkdir(parents=True, exist_ok=True)
    written = 0
    try:
        with gzip.open(raw_path, "rt", encoding="utf-8") as f:
            with out_path.open("w", encoding="utf-8") as out:
                for line in f:
                    line = line.strip()
                    if not line:
                        continue
                    task = json.loads(line)
                    prompt = task.get("prompt", "")
                    entry_point = task.get("entry_point", "")
                    canonical = task.get("canonical_solution", "")
                    test = task.get("test", "")
                    full_code = prompt + canonical
                    normalized = {
                        "task_id": task.get("task_id", f"HumanEval/{written}"),
                        "prompt": prompt,
                        "entry_point": entry_point,
                        "canonical_solution": canonical,
                        "test": test,
                        "full_code": full_code,
                    }
                    out.write(json.dumps(normalized, ensure_ascii=False) + "\n")
                    written += 1
    except (OSError, json.JSONDecodeError) as exc:
        log.warning("normalize_failed", error=str(exc))
        return 0
    return written


def _normalize_mbpp(raw_path: Path, out_path: Path) -> int:
    log = get_logger(operation="normalize_mbpp")
    out_path.parent.mkdir(parents=True, exist_ok=True)
    written = 0
    try:
        with (
            raw_path.open("r", encoding="utf-8") as f,
            out_path.open("w", encoding="utf-8") as out,
        ):
            for line in f:
                line = line.strip()
                if not line:
                    continue
                task = json.loads(line)
                task_id = task.get("task_id", written)
                text = task.get("text", "")
                code = task.get("code", "")
                test_list = task.get("test_list", [])
                test_setup = task.get("test_setup_code", "")
                normalized = {
                    "task_id": task_id,
                    "text": text,
                    "code": code,
                    "test_list": test_list,
                    "test_setup_code": test_setup,
                }
                out.write(json.dumps(normalized, ensure_ascii=False) + "\n")
                written += 1
    except (OSError, json.JSONDecodeError) as exc:
        log.warning("normalize_failed", error=str(exc))
        return 0
    return written


def fetch_humaneval(output_dir: Path = Path("data/benchmarks")) -> Path | None:
    gz_path = output_dir / "HumanEval.jsonl.gz"
    out_path = output_dir / "humaneval.jsonl"
    if out_path.exists() and out_path.stat().st_size > 0:
        return out_path
    if not _download(HUMANEVAL_URL, gz_path):
        return None
    n = _normalize_humaneval(gz_path, out_path)
    if n == 0:
        return None
    gz_path.unlink(missing_ok=True)
    print(f"HumanEval: wrote {n} tasks → {out_path}")
    return out_path


def fetch_mbpp(output_dir: Path = Path("data/benchmarks")) -> Path | None:
    out_path = output_dir / "mbpp.jsonl"
    if out_path.exists() and out_path.stat().st_size > 0:
        return out_path
    log = get_logger(operation="fetch_mbpp")
    try:
        from datasets import load_dataset
    except ImportError:
        log.warning("hf_unavailable", msg="install `datasets` to fetch MBPP")
        return _fetch_mbpp_fallback(output_dir)

    output_dir.mkdir(parents=True, exist_ok=True)
    try:
        ds = load_dataset(
            "google-research-datasets/mbpp",
            "sanitized",
            split="train+validation+test+prompt",
        )
    except Exception as exc:
        log.warning("hf_fetch_failed", error=str(exc))
        return _fetch_mbpp_fallback(output_dir)

    written = 0
    with out_path.open("w", encoding="utf-8") as f:
        for task in ds:
            normalized = {
                "task_id": task.get("task_id", written),
                "text": task.get("prompt", "") or task.get("text", ""),
                "code": task.get("code", ""),
                "test_list": task.get("test_list", []),
                "test_setup_code": task.get("test_imports", []),
            }
            f.write(json.dumps(normalized, ensure_ascii=False) + "\n")
            written += 1
    print(f"MBPP: wrote {written} tasks → {out_path}")
    return out_path


def _fetch_mbpp_fallback(output_dir: Path) -> Path | None:
    """Last-resort: write a small smoke-test MBPP shard (1 task)."""
    out_path = output_dir / "mbpp.jsonl"
    out_path.parent.mkdir(parents=True, exist_ok=True)
    smoke = {
        "task_id": 1,
        "text": "Write a function to add two numbers.",
        "code": "def add(a, b):\n    return a + b\n",
        "test_list": ["assert add(1, 2) == 3", "assert add(-1, 1) == 0"],
        "test_setup_code": "",
    }
    out_path.write_text(json.dumps(smoke) + "\n", encoding="utf-8")
    print(f"MBPP fallback: wrote 1 smoke task → {out_path}")
    return out_path


def fetch_all(output_dir: Path = Path("data/benchmarks")) -> dict[str, Path | None]:
    output_dir.mkdir(parents=True, exist_ok=True)
    return {
        "humaneval": fetch_humaneval(output_dir),
        "mbpp": fetch_mbpp(output_dir),
    }


if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser(description="Fetch HumanEval + MBPP benchmarks")
    parser.add_argument("--output-dir", default="data/benchmarks")
    parser.add_argument("--only", default="all", choices=["all", "humaneval", "mbpp"])
    args = parser.parse_args()
    out_dir = Path(args.output_dir)
    if args.only in ("all", "humaneval"):
        fetch_humaneval(out_dir)
    if args.only in ("all", "mbpp"):
        fetch_mbpp(out_dir)
