"""Difficulty scoring for curriculum learning.

Combines multiple signals into a single 0–1 difficulty score per row:
- Output length (longer outputs = harder tasks on average)
- Output complexity: AST node count (Python) / brace depth (others)
- Code block density
- Vocabulary richness (type-token ratio)
- Domain-weighted hints (e.g. "advanced" tags)
- Optional exec pass-rate on a small held-out set (offline path)
"""

from __future__ import annotations

import ast
import json
import re
from dataclasses import dataclass
from pathlib import Path
from typing import Optional

from remllm.data.loader import load_jsonl


_VOCAB_RE = re.compile(r"[A-Za-z_][A-Za-z0-9_]{2,}")
_PY_KEYWORDS = {
    "lambda",
    "yield",
    "async",
    "await",
    "decorator",
    "metaclass",
    "generator",
    "contextmanager",
    "abstractmethod",
}
_JS_KEYWORDS = {
    "async",
    "await",
    "promise",
    "closure",
    "prototype",
    "decorator",
}


def _python_complexity(code: str) -> int:
    try:
        tree = ast.parse(code)
    except (SyntaxError, ValueError):
        return 0
    nodes = list(ast.walk(tree))
    return len(nodes) + sum(
        2 for n in nodes if isinstance(n, (ast.Lambda, ast.Yield, ast.AsyncFor))
    )


def _brace_depth(code: str) -> int:
    depth = 0
    max_depth = 0
    for ch in code:
        if ch == "{":
            depth += 1
            max_depth = max(max_depth, depth)
        elif ch == "}":
            depth = max(0, depth - 1)
    return max_depth


def _vocab_richness(text: str) -> float:
    tokens = _VOCAB_RE.findall(text)
    if not tokens:
        return 0.0
    return len(set(tokens)) / len(tokens)


def _code_blocks(output: str) -> list[str]:
    return re.findall(r"```(?:[a-zA-Z0-9_+-]+)?\n(.*?)```", output, flags=re.DOTALL)


def score_difficulty(row: dict, max_length: int = 10000) -> float:
    """Return a 0..1 difficulty score for a row."""
    instruction = str(row.get("instruction", ""))
    user_input = str(row.get("input", ""))
    output = str(row.get("output", ""))

    out_len = min(len(output), max_length)
    length_score = out_len / max_length

    code_blocks = _code_blocks(output)
    code_density = min(1.0, sum(len(b) for b in code_blocks) / max(1, out_len))

    complexity = 0
    for block in code_blocks:
        if "def " in block or "class " in block or "import " in block:
            complexity = max(complexity, _python_complexity(block))
        else:
            complexity = max(complexity, _brace_depth(block) * 3)
    complexity_score = min(1.0, complexity / 80.0)

    vocab_score = _vocab_richness(output + " " + instruction)

    instr_lower = instruction.lower()
    keyword_score = 0.0
    for kw in _PY_KEYWORDS.union(_JS_KEYWORDS):
        if kw in instr_lower:
            keyword_score += 0.15
    keyword_score = min(1.0, keyword_score)

    has_input_ctx = 0.1 if user_input.strip() else 0.0

    score = (
        0.30 * length_score
        + 0.25 * complexity_score
        + 0.15 * code_density
        + 0.15 * vocab_score
        + 0.10 * keyword_score
        + 0.05 * has_input_ctx
    )
    return round(max(0.0, min(1.0, score)), 4)


@dataclass
class DifficultyBands:
    easy_max: float = 0.33
    intermediate_max: float = 0.66

    def classify(self, score: float) -> str:
        if score < self.easy_max:
            return "easy"
        if score < self.intermediate_max:
            return "intermediate"
        return "advanced"


def annotate_difficulty(
    input_path: Path,
    output_path: Path,
    bands: Optional[DifficultyBands] = None,
    adaptive: bool = True,
) -> dict:
    """Score every row in `input_path` and write to `output_path` with a
    `difficulty` field added (or updated).

    If `adaptive=True`, the bands are re-derived from the score distribution
    (33rd/66th percentiles) so each bucket contains ~1/3 of rows.
    """
    rows = load_jsonl(input_path)
    scores = [score_difficulty(r) for r in rows]
    if adaptive and rows:
        sorted_scores = sorted(scores)
        n = len(sorted_scores)
        e1 = sorted_scores[n // 3]
        e2 = sorted_scores[(2 * n) // 3]
        bands = DifficultyBands(
            easy_max=round(e1, 4),
            intermediate_max=round(e2, 4),
        )
    else:
        bands = bands or DifficultyBands()

    annotated: list[dict] = []
    distribution = {"easy": 0, "intermediate": 0, "advanced": 0}
    for row, s in zip(rows, scores):
        row["difficulty_score"] = s
        row["difficulty"] = bands.classify(s)
        distribution[row["difficulty"]] += 1
        annotated.append(row)

    output_path.parent.mkdir(parents=True, exist_ok=True)
    with output_path.open("w", encoding="utf-8") as f:
        for row in annotated:
            f.write(json.dumps(row, ensure_ascii=False) + "\n")

    stats = {
        "input_rows": len(rows),
        "output_rows": len(annotated),
        "difficulty_distribution": distribution,
        "bands": {
            "easy_max": bands.easy_max,
            "intermediate_max": bands.intermediate_max,
            "adaptive": adaptive,
        },
        "score_stats": {
            "min": min(scores) if scores else 0,
            "max": max(scores) if scores else 0,
            "median": sorted(scores)[len(scores) // 2] if scores else 0,
        },
    }
    return stats
