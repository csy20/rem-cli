"""Perplexity-based data filtering for training data quality.

Two tiers:
- `filter_heuristic` — local, fast, scales to millions of rows. Uses length,
  repetition, code-block heuristics, and language consistency. No LLM calls.
- `filter_by_perplexity` — Ollama-based, slow. Use as a final-stage gate on
  the top-N candidates.
"""

import json
import math
import re
import subprocess
from pathlib import Path
from typing import Any, Iterator


_HEURISTIC_REPEAT_RE = re.compile(r"(.)\1{8,}")  # 8+ repeated chars
_HEURISTIC_LINE_REPEAT_RE = re.compile(r"^(.+)$\n^(?:\1)$", re.MULTILINE)
_CODE_BLOCK_RE = re.compile(r"```[a-zA-Z]*\n.*?```", re.DOTALL)


def heuristic_quality_score(row: dict) -> float:
    """Return a 0–10 quality score based on local heuristics only.

    Signals (each contributes to the final score):
    - output length (sweet-spot 200–3000 chars)
    - instruction length (≥ 12 chars)
    - non-repetitive content (penalty for long runs of one char / repeated lines)
    - presence of code blocks for code-related prompts
    - language coherence (instruction and output share vocabulary)
    """
    instruction = str(row.get("instruction", "")).strip()
    user_input = str(row.get("input", "")).strip()
    output = str(row.get("output", "")).strip()

    if not instruction or not output:
        return 0.0

    score = 5.0

    out_len = len(output)
    if out_len < 50:
        score -= 2.0
    elif out_len < 200:
        score -= 0.5
    elif 200 <= out_len <= 3000:
        score += 1.5
    elif out_len <= 8000:
        score += 0.5
    else:
        score -= 1.0  # very long, often bloated / hallucinated

    if len(instruction) < 12:
        score -= 1.5
    if user_input and len(user_input) > 5000:
        score -= 0.5

    if _HEURISTIC_REPEAT_RE.search(output):
        score -= 2.5
    if _HEURISTIC_LINE_REPEAT_RE.search(output):
        score -= 1.0

    has_code = bool(_CODE_BLOCK_RE.search(output)) or "```" in output
    instr_lower = instruction.lower()
    code_intent = any(
        kw in instr_lower
        for kw in (
            "code",
            "function",
            "implement",
            "write",
            "create",
            "fix",
            "refactor",
        )
    )
    if code_intent and not has_code:
        score -= 2.0
    if has_code and not code_intent:
        score -= 0.5

    words_out = set(output.lower().split())
    words_in = set(instruction.lower().split())
    if words_in:
        overlap = len(words_in & words_out) / max(1, len(words_in))
        if overlap < 0.02 and has_code:
            score -= 0.5

    return max(0.0, min(10.0, score))


def filter_heuristic(
    input_path: Path,
    output_path: Path,
    threshold: float = 5.0,
) -> dict:
    """Apply local-heuristic quality filter. Fast and offline."""
    from remllm.data.loader import load_jsonl, write_jsonl

    rows = load_jsonl(input_path)
    original = len(rows)
    passed = []
    removed = 0
    score_buckets = {"0-2": 0, "2-4": 0, "4-6": 0, "6-8": 0, "8-10": 0}
    for row in rows:
        s = heuristic_quality_score(row)
        bucket = (
            "0-2"
            if s < 2
            else "2-4"
            if s < 4
            else "4-6"
            if s < 6
            else "8-10"
            if s >= 8
            else "6-8"
        )
        score_buckets[bucket] += 1
        if s >= threshold:
            passed.append(row)
        else:
            removed += 1

    write_jsonl(output_path, passed)
    stats = {
        "original": original,
        "removed_low_quality": removed,
        "remaining": len(passed),
        "threshold": threshold,
        "method": "heuristic",
        "score_distribution": score_buckets,
    }
    print(json.dumps(stats, indent=2))
    return stats


def filter_stream_heuristic(
    rows: Iterator[dict[str, Any]], threshold: float = 5.0
) -> Iterator[dict[str, Any]]:
    """Streaming variant of heuristic filter."""
    for row in rows:
        if heuristic_quality_score(row) >= threshold:
            yield row


def compute_perplexity_ollama(
    text: str,
    model: str = "qwen2.5-coder:1.5b",
    timeout_s: int = 60,
) -> float:
    eval_prompt = (
        f"Rate the quality of this training example on a scale of 0-10, "
        f"where 10 is excellent. Reply with ONLY the number.\n\n"
        f"Example:\n```\n{text[:2000]}\n```\n\nRating:"
    )
    result = subprocess.run(
        ["ollama", "run", model, eval_prompt],
        capture_output=True,
        text=True,
        timeout=timeout_s,
    )
    if result.returncode != 0:
        return 5.0
    raw = result.stdout.strip()
    try:
        score = float(raw.split()[0]) if raw else 5.0
        return score
    except ValueError:
        for word in raw.split():
            try:
                return float(word)
            except ValueError:
                continue
        return 5.0


def filter_by_perplexity(
    input_path: Path,
    output_path: Path,
    model: str = "qwen2.5-coder:1.5b",
    threshold: float = 5.0,
    max_samples: int = 0,
    timeout_s: int = 60,
) -> dict:
    from remllm.data.loader import load_jsonl, write_jsonl

    rows = load_jsonl(input_path)
    original = len(rows)

    passed = []
    removed = 0
    scored = 0

    sample_count = min(max_samples, len(rows)) if max_samples > 0 else len(rows)

    for i, row in enumerate(rows[:sample_count]):
        text = row.get("output", "") or row.get("instruction", "")
        score = compute_perplexity_ollama(text, model=model, timeout_s=timeout_s)
        scored += 1
        if score >= threshold:
            passed.append(row)
        else:
            removed += 1
            print(
                f"  Filtered: score={score:.1f} threshold={threshold} — {str(row.get('instruction', ''))[:80]}"
            )

    if sample_count < len(rows):
        passed.extend(rows[sample_count:])

    write_jsonl(output_path, passed)
    stats = {
        "original": original,
        "scored": scored,
        "removed_low_quality": removed,
        "remaining": len(passed),
        "threshold": threshold,
        "method": "ollama",
    }
    print(json.dumps(stats, indent=2))
    return stats
