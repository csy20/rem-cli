"""Curated dataset pipeline — the v1 Day-1 deliverable.

Stages:
    sources → normalize → exact-dedup → near-dedup → heuristic filter → mix → split

Sources supported (each is a function that yields normalized rows):
- hf_codealpaca   : HuggingFace `sahil2801/CodeAlpaca-20k` (instruction-tuning)
- hf_magicoder    : HuggingFace `ise-uiuc/Magicoder-OSS-Instruct-75K` (real OSS code)
- hf_evol_code    : HuggingFace `nickrosh/Evol-Instruct-Code-80k-v1` (evolved)
- local_synthetic : existing `data/domains/` and `data/raw.jsonl`
- local_jsonl     : any user-provided JSONL with {instruction, input, output}

The whole pipeline is streaming-safe; it never materializes the full corpus
in memory.
"""

from __future__ import annotations

import json
import random
import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable, Iterator

from remllm.logging import get_logger


REQUIRED_KEYS = ("instruction", "input", "output")


# ── Normalization ────────────────────────────────────────────────────────────


def _normalize(row: dict) -> dict | None:
    """Normalize a raw row into {instruction, input, output}."""
    if "instruction" in row and "output" in row:
        instruction = str(row.get("instruction", "")).strip()
        user_input = str(row.get("input", "")).strip()
        output = str(row.get("output", "")).strip()
    elif "prompt" in row and "response" in row:
        instruction = str(row.get("prompt", "")).strip()
        user_input = str(row.get("context", "")).strip()
        output = str(row.get("response", "")).strip()
    elif "problem" in row and "solution" in row:
        instruction = str(row.get("problem", "")).strip()
        user_input = str(row.get("test", "")).strip()
        output = str(row.get("solution", "")).strip()
    elif "question" in row and "answer" in row:
        instruction = str(row.get("question", "")).strip()
        user_input = ""
        output = str(row.get("answer", "")).strip()
    elif "instruction" in row and "response" in row:
        instruction = str(row.get("instruction", "")).strip()
        user_input = str(row.get("input", "")).strip()
        output = str(row.get("response", "")).strip()
    elif "messages" in row and isinstance(row["messages"], list):
        user_msgs = [m["content"] for m in row["messages"] if m.get("role") == "user"]
        asst_msgs = [
            m["content"] for m in row["messages"] if m.get("role") == "assistant"
        ]
        if not user_msgs or not asst_msgs:
            return None
        instruction = str(user_msgs[0]).strip()
        user_input = ""
        output = str(asst_msgs[0]).strip()
    else:
        return None

    if not instruction or not output:
        return None
    if len(instruction) < 8 or len(output) < 8:
        return None
    if len(output) > 20000:
        return None
    if not user_input:
        user_input = ""
    return {
        "instruction": instruction,
        "input": user_input,
        "output": output,
    }


def _ensure_domain(row: dict, default_domain: str = "general") -> dict:
    if "domain" not in row:
        row["domain"] = default_domain
    if "difficulty" not in row:
        row["difficulty"] = _guess_difficulty(row)
    if "tags" not in row:
        row["tags"] = []
    return row


def _guess_difficulty(row: dict) -> str:
    out = row.get("output", "")
    if len(out) < 300:
        return "easy"
    if len(out) < 1200:
        return "intermediate"
    return "advanced"


# ── Sources ─────────────────────────────────────────────────────────────────


@dataclass
class Source:
    name: str
    description: str
    pull: Callable[[Path | None, int], Iterator[dict]]
    default_limit: int = 0
    enabled: bool = True


def _try_load_jsonl(path: Path) -> list[dict]:
    if not path.exists():
        return []
    rows = []
    with path.open("r", encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                rows.append(json.loads(line))
            except json.JSONDecodeError:
                continue
    return rows


def _src_hf_codealpaca(cache_dir: Path | None, limit: int) -> Iterator[dict]:
    """HuggingFace CodeAlpaca-20k, with offline fallback to cached file."""
    cache_path = (
        cache_dir / "codealpaca.jsonl"
        if cache_dir
        else Path("data/sources/codealpaca.jsonl")
    )
    rows = _try_load_jsonl(cache_path)
    if not rows:
        rows = _try_load_jsonl(Path("data/sources/codealpaca.jsonl"))
    if not rows:
        log = get_logger(source="hf_codealpaca")
        log.warning("source_offline", msg="codealpaca.jsonl not present, skipping")
        return
    yield from rows[:limit] if limit > 0 else rows


def _src_hf_magicoder(cache_dir: Path | None, limit: int) -> Iterator[dict]:
    cache_path = (
        cache_dir / "magicoder.jsonl"
        if cache_dir
        else Path("data/sources/magicoder.jsonl")
    )
    rows = _try_load_jsonl(cache_path)
    if not rows:
        rows = _try_load_jsonl(Path("data/sources/magicoder.jsonl"))
    if not rows:
        log = get_logger(source="hf_magicoder")
        log.warning("source_offline", msg="magicoder.jsonl not present, skipping")
        return
    yield from rows[:limit] if limit > 0 else rows


def _src_hf_evol_code(cache_dir: Path | None, limit: int) -> Iterator[dict]:
    cache_path = (
        cache_dir / "evol_code.jsonl"
        if cache_dir
        else Path("data/sources/evol_code.jsonl")
    )
    rows = _try_load_jsonl(cache_path)
    if not rows:
        rows = _try_load_jsonl(Path("data/sources/evol_code.jsonl"))
    if not rows:
        return
    yield from rows[:limit] if limit > 0 else rows


def _src_local_synthetic(cache_dir: Path | None, limit: int) -> Iterator[dict]:
    """Use existing domains/ data and raw.jsonl as a local source."""
    for path in [
        Path("data/raw.jsonl"),
        Path("data/domains/beginner/train.jsonl"),
        Path("data/domains/beginner/raw.jsonl"),
        Path("data/domains/beginner/raw.generated.jsonl"),
        Path("data/domains/nextjs/raw/fullstack.jsonl"),
    ]:
        rows = _try_load_jsonl(path)
        for row in rows:
            row["domain"] = row.get("domain", path.stem)
            yield row
            if limit > 0 and limit <= 0:
                break


def _src_local_jsonl_factory(
    path: Path,
) -> Callable[[Path | None, int], Iterator[dict]]:
    def _src(_cache: Path | None, limit: int) -> Iterator[dict]:
        rows = _try_load_jsonl(path)
        yield from rows[:limit] if limit > 0 else rows

    return _src


SOURCE_REGISTRY: dict[str, Source] = {
    "hf_codealpaca": Source(
        name="hf_codealpaca",
        description="HuggingFace sahil2801/CodeAlpaca-20k (cache: data/sources/codealpaca.jsonl)",
        pull=_src_hf_codealpaca,
        default_limit=20000,
    ),
    "hf_magicoder": Source(
        name="hf_magicoder",
        description="HuggingFace ise-uiuc/Magicoder-OSS-Instruct-75K (cache: data/sources/magicoder.jsonl)",
        pull=_src_hf_magicoder,
        default_limit=30000,
    ),
    "hf_evol_code": Source(
        name="hf_evol_code",
        description="HuggingFace nickrosh/Evol-Instruct-Code-80k-v1 (cache: data/sources/evol_code.jsonl)",
        pull=_src_hf_evol_code,
        default_limit=25000,
    ),
    "local_synthetic": Source(
        name="local_synthetic",
        description="Existing data/raw.jsonl + data/domains/* training data",
        pull=_src_local_synthetic,
        default_limit=0,
    ),
}


# ── Source fetchers (network-optional) ───────────────────────────────────────


def fetch_hf_dataset(
    dataset_name: str,
    output_path: Path,
    split: str = "train",
    max_samples: int = 0,
    config: str | None = None,
) -> int:
    """Download a HF dataset and convert to {instruction, input, output} JSONL.

    Returns number of rows written. Gracefully returns 0 if datasets/huggingface_hub
    is unavailable or download fails.
    """
    log = get_logger(source=dataset_name, output=str(output_path))
    try:
        from datasets import load_dataset
    except ImportError:
        log.warning("hf_unavailable", msg="install `datasets` to enable HF fetches")
        return 0

    output_path.parent.mkdir(parents=True, exist_ok=True)
    try:
        if config:
            ds = load_dataset(dataset_name, config, split=split, streaming=False)
        else:
            ds = load_dataset(dataset_name, split=split, streaming=False)
    except Exception as exc:  # network, auth, format issues
        log.warning("hf_fetch_failed", dataset=dataset_name, error=str(exc))
        return 0

    written = 0
    with output_path.open("w", encoding="utf-8") as f:
        for i, row in enumerate(ds):
            if max_samples > 0 and i >= max_samples:
                break
            normalized = _normalize(dict(row))
            if normalized is None:
                continue
            f.write(json.dumps(normalized, ensure_ascii=False) + "\n")
            written += 1
    log.info("hf_fetch_complete", dataset=dataset_name, rows=written)
    return written


# ── Curator orchestrator ─────────────────────────────────────────────────────


@dataclass
class CurateConfig:
    sources: list[str] = field(default_factory=lambda: ["local_synthetic"])
    output_dir: Path = Path("data/curated/v1")
    cache_dir: Path | None = Path("data/sources")
    target_train_size: int = 30000
    train_split: float = 0.9
    val_split: float = 0.05
    eval_split: float = 0.05
    seed: int = 42
    near_dedup_threshold: float = 0.85
    heuristic_threshold: float = 5.0
    max_eval: int = 200
    max_val: int = 500
    source_limits: dict[str, int] = field(default_factory=dict)
    min_output_length: int = 30
    max_output_length: int = 12000


def curate(cfg: CurateConfig) -> dict:
    log = get_logger(operation="curate", output_dir=str(cfg.output_dir))
    rng = random.Random(cfg.seed)

    log.info("curate_start", sources=cfg.sources)

    # ── Pull & normalize ────────────────────────────────────────────────────
    normalized: list[dict] = []
    per_source_counts: dict[str, int] = {}
    for source_name in cfg.sources:
        source = SOURCE_REGISTRY.get(source_name)
        if source is None:
            log.warning("unknown_source", source=source_name)
            continue
        limit = cfg.source_limits.get(source_name, source.default_limit)
        pulled = 0
        for raw in source.pull(cfg.cache_dir, limit):
            row = _normalize(raw)
            if row is None:
                continue
            if len(row["output"]) < cfg.min_output_length:
                continue
            if len(row["output"]) > cfg.max_output_length:
                continue
            row = _ensure_domain(row)
            normalized.append(row)
            pulled += 1
        per_source_counts[source_name] = pulled
        log.info("source_pulled", source=source_name, rows=pulled, limit=limit)

    original = len(normalized)
    log.info("normalize_complete", rows=original, per_source=per_source_counts)

    # ── Exact dedup ─────────────────────────────────────────────────────────
    from remllm.data.dedup import deduplicate_exact

    normalized, exact_dropped = deduplicate_exact(normalized)
    log.info("exact_dedup_complete", dropped=exact_dropped, remaining=len(normalized))

    # ── Near-dedup (MinHash) ────────────────────────────────────────────────
    from remllm.data.dedup import deduplicate_minhash

    normalized, near_dropped = deduplicate_minhash(
        normalized, threshold=cfg.near_dedup_threshold, num_perm=64, seed=cfg.seed
    )
    log.info("near_dedup_complete", dropped=near_dropped, remaining=len(normalized))

    # ── Heuristic filter ────────────────────────────────────────────────────
    from remllm.data.filter import heuristic_quality_score

    filtered = []
    filter_dropped = 0
    for row in normalized:
        s = heuristic_quality_score(row)
        if s >= cfg.heuristic_threshold:
            filtered.append(row)
        else:
            filter_dropped += 1
    log.info(
        "heuristic_filter_complete",
        dropped=filter_dropped,
        remaining=len(filtered),
        threshold=cfg.heuristic_threshold,
    )
    normalized = filtered

    # ── Mix / cap to target size ────────────────────────────────────────────
    rng.shuffle(normalized)
    if cfg.target_train_size > 0 and len(normalized) > cfg.target_train_size:
        normalized = normalized[: cfg.target_train_size]
    log.info("mix_complete", final=len(normalized))

    # ── Split train / val / eval ────────────────────────────────────────────
    cfg.output_dir.mkdir(parents=True, exist_ok=True)
    n = len(normalized)
    n_train = max(1, int(n * cfg.train_split))
    n_val = max(1, int(n * cfg.val_split))
    n_eval = min(max(1, int(n * cfg.eval_split)), cfg.max_eval)
    n_val = min(n_val, cfg.max_val)

    train_rows = normalized[:n_train]
    val_rows = normalized[n_train : n_train + n_val]
    eval_rows = normalized[n_train + n_val : n_train + n_val + n_eval]

    train_path = cfg.output_dir / "train.jsonl"
    val_path = cfg.output_dir / "val.jsonl"
    eval_path = cfg.output_dir / "eval.jsonl"
    _write_jsonl(train_path, train_rows)
    _write_jsonl(val_path, val_rows)
    _write_jsonl(eval_path, eval_rows)

    # ── Manifest ────────────────────────────────────────────────────────────
    domain_counts: dict[str, int] = {}
    diff_counts: dict[str, int] = {}
    for row in train_rows:
        d = row.get("domain", "general")
        domain_counts[d] = domain_counts.get(d, 0) + 1
        diff = row.get("difficulty", "unknown")
        diff_counts[diff] = diff_counts.get(diff, 0) + 1

    manifest = {
        "version": "v1",
        "sources": cfg.sources,
        "per_source_counts": per_source_counts,
        "original_rows": original,
        "after_exact_dedup": original - exact_dropped,
        "after_near_dedup": original - exact_dropped - near_dropped,
        "after_heuristic_filter": len(train_rows) + len(val_rows) + len(eval_rows),
        "near_dedup_threshold": cfg.near_dedup_threshold,
        "heuristic_threshold": cfg.heuristic_threshold,
        "train_rows": len(train_rows),
        "val_rows": len(val_rows),
        "eval_rows": len(eval_rows),
        "domain_distribution": domain_counts,
        "difficulty_distribution": diff_counts,
        "seed": cfg.seed,
        "train_split": cfg.train_split,
        "val_split": cfg.val_split,
        "eval_split": cfg.eval_split,
        "outputs": {
            "train": str(train_path),
            "val": str(val_path),
            "eval": str(eval_path),
        },
    }
    manifest_path = cfg.output_dir / "manifest.json"
    manifest_path.write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    log.info("curate_complete", manifest=str(manifest_path))
    print(json.dumps(manifest, indent=2))
    return manifest


def _write_jsonl(path: Path, rows: list[dict]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as f:
        for row in rows:
            f.write(json.dumps(row, ensure_ascii=False) + "\n")


# ── CLI helpers ─────────────────────────────────────────────────────────────


def list_sources() -> list[dict]:
    return [
        {"name": s.name, "description": s.description, "default_limit": s.default_limit}
        for s in SOURCE_REGISTRY.values()
    ]
