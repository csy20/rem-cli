"""Balanced data mixing — combines multiple domain-specific datasets
into a single balanced training file using weighted random sampling."""

import json
import random
from pathlib import Path


DEFAULT_MIX_RATIOS = {
    "code_gen": 0.40,
    "fim": 0.20,
    "bug_fix": 0.15,
    "explain": 0.10,
    "refactor": 0.10,
    "conversation": 0.05,
}


def mix_datasets(
    dataset_paths: dict[str, Path],
    mix_ratios: dict[str, float] | None = None,
    output_path: Path | None = None,
    target_size: int = 0,
    seed: int = 42,
) -> list[dict]:
    from remllm.data.loader import load_jsonl, write_jsonl

    ratios = mix_ratios or DEFAULT_MIX_RATIOS
    random.seed(seed)

    pool: dict[str, list[dict]] = {}
    for name, path in dataset_paths.items():
        if path.exists():
            pool[name] = load_jsonl(path)

    if not pool:
        print("No datasets found")
        return []

    total_ratio = sum(ratios.get(k, 0) for k in pool)
    if total_ratio == 0:
        if not pool:
            return []
        max_size = max(len(v) for v in pool.values())
        combined = []
        for rows in pool.values():
            combined.extend(rows[:max_size])
        random.shuffle(combined)
        return combined

    max_requested = 0
    for name, rows in pool.items():
        ratio = ratios.get(name, 0) / total_ratio
        needed = int(len(rows) / ratio) if ratio > 0 else len(pool[name])
        max_requested = max(max_requested, needed)

    target = target_size if target_size > 0 else max_requested
    combined = []
    stats = {}

    for name, rows in pool.items():
        ratio = ratios.get(name, 0) / total_ratio
        n_samples = max(1, int(target * ratio))
        n_samples = min(n_samples, len(rows))
        sampled = (
            random.sample(rows, n_samples) if n_samples < len(rows) else list(rows)
        )
        combined.extend(sampled)
        stats[name] = {
            "ratio": round(ratio, 3),
            "sampled": n_samples,
            "available": len(rows),
        }

    random.shuffle(combined)
    print(json.dumps(stats, indent=2))

    if output_path:
        write_jsonl(output_path, combined)
        print(f"Mixed {len(combined)} rows from {len(pool)} datasets → {output_path}")

    return combined
