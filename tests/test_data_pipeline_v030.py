"""Regression tests for the v0.3.0 data pipeline (Day 1)."""

import json
import tempfile
from pathlib import Path

import pytest

from remllm.data.curator import (
    SOURCE_REGISTRY,
    CurateConfig,
    _normalize,
    curate,
)
from remllm.data.dedup import (
    deduplicate_exact,
    deduplicate_minhash,
    deduplicate_near,
    stream_deduplicate_exact,
    stream_deduplicate_minhash,
)
from remllm.data.difficulty import (
    DifficultyBands,
    annotate_difficulty,
    score_difficulty,
)
from remllm.data.filter import (
    filter_heuristic,
    filter_stream_heuristic,
    heuristic_quality_score,
)


# ── normalize ──────────────────────────────────────────────────────────────


def test_normalize_instruction_output():
    row = {"instruction": "do something", "input": "", "output": "X is done now"}
    assert _normalize(row) == {
        "instruction": "do something",
        "input": "",
        "output": "X is done now",
    }


def test_normalize_prompt_response():
    row = {"prompt": "do something", "context": "ctx", "response": "X is done now"}
    assert _normalize(row) == {
        "instruction": "do something",
        "input": "ctx",
        "output": "X is done now",
    }


def test_normalize_problem_solution():
    row = {
        "problem": "compute pi",
        "test": "t",
        "solution": "import math; pi = math.pi",
    }
    assert _normalize(row) == {
        "instruction": "compute pi",
        "input": "t",
        "output": "import math; pi = math.pi",
    }


def test_normalize_question_answer():
    row = {"question": "what is 2+2", "answer": "the answer is four"}
    assert _normalize(row) == {
        "instruction": "what is 2+2",
        "input": "",
        "output": "the answer is four",
    }


def test_normalize_messages():
    row = {
        "messages": [
            {"role": "user", "content": "do thing"},
            {"role": "assistant", "content": "thing done"},
        ]
    }
    assert _normalize(row) == {
        "instruction": "do thing",
        "input": "",
        "output": "thing done",
    }


def test_normalize_rejects_short():
    assert _normalize({"instruction": "x", "output": "y"}) is None
    assert _normalize({"instruction": "", "output": "valid output text"}) is None


def test_normalize_rejects_overlong():
    assert _normalize({"instruction": "x", "output": "a" * 30000}) is None


def test_normalize_handles_unknown_schema():
    assert _normalize({"foo": "bar"}) is None


# ── dedup ──────────────────────────────────────────────────────────────────


def test_dedup_exact_basic():
    rows = [{"a": 1}, {"a": 2}, {"a": 1}, {"a": 3}, {"a": 2}]
    dedup, dropped = deduplicate_exact(rows)
    assert dropped == 2
    assert len(dedup) == 3
    assert [r["a"] for r in dedup] == [1, 2, 3]


def test_dedup_exact_streaming_matches_batch():
    rows = [{"a": i % 5} for i in range(50)]
    batch_dedup, _ = deduplicate_exact(rows)
    stream_dedup = list(stream_deduplicate_exact(iter(rows)))
    assert len(batch_dedup) == len(stream_dedup)
    assert [r["a"] for r in batch_dedup] == [r["a"] for r in stream_dedup]


def test_dedup_minhash_catches_near_dupes():
    base = "the quick brown fox jumps over the lazy dog " * 20
    near = "the quick brown fox jumps over the lazy cat " * 20
    other = "completely different content about Python programming " * 20
    rows = [
        {"output": base},
        {"output": near},
        {"output": other},
    ]
    dedup, dropped = deduplicate_minhash(rows, threshold=0.7, num_perm=64, bands=16)
    assert dropped == 1
    assert len(dedup) == 2


def test_dedup_minhash_streaming():
    base = "alpha beta gamma delta " * 30
    near = "alpha beta gamma epsilon " * 30
    rows_iter = iter([{"output": base}, {"output": near}, {"output": base}])
    kept = list(stream_deduplicate_minhash(rows_iter, threshold=0.7))
    assert len(kept) == 2


def test_dedup_near_legacy_path():
    rows = [
        {"output": "Python function to sort a list of integers"},
        {"output": "Python function to sort a list of floats"},
    ]
    dedup, dropped = deduplicate_near(rows, threshold=0.5)
    assert dropped >= 1
    assert len(dedup) == 1


# ── heuristic filter ───────────────────────────────────────────────────────


def test_heuristic_scores_zero_for_empty():
    assert heuristic_quality_score({"instruction": "", "output": ""}) == 0.0
    assert heuristic_quality_score({}) == 0.0


def test_heuristic_scores_higher_for_good_code():
    good = {
        "instruction": "Write a Python function to compute factorial",
        "input": "",
        "output": "```python\ndef factorial(n):\n    if n <= 1:\n        return 1\n    return n * factorial(n - 1)\n```",
    }
    bad = {
        "instruction": "Write a Python function to compute factorial",
        "input": "",
        "output": "a" * 200,
    }
    assert heuristic_quality_score(good) > heuristic_quality_score(bad)


def test_heuristic_penalizes_repetition():
    row = {
        "instruction": "Implement fizzbuzz",
        "output": "x" * 200,
    }
    assert heuristic_quality_score(row) < 5.0


def test_heuristic_penalizes_missing_code_for_code_intent():
    row = {
        "instruction": "Write a function to add two numbers",
        "output": "Sure, just call a built-in add function that exists in Python.",
    }
    s = heuristic_quality_score(row)
    assert s < 5.0


def test_filter_heuristic_filters_low_quality():
    rows = [
        {"instruction": "Write a function", "output": "ok"},  # short
        {
            "instruction": "Write a Python function to sort a list of integers and return it",
            "output": (
                "You can use Python's built-in `sorted` function. Here is a complete "
                "example with docstring, type hints, and edge-case handling:\n\n"
                "```python\n"
                "from typing import List\n\n"
                "def sort_list(xs: List[int]) -> List[int]:\n"
                '    """Return a new sorted list."""\n'
                "    return sorted(xs)\n"
                "```\n\n"
                "This handles any iterable of integers and returns a new list."
            ),
        },
    ]
    with tempfile.TemporaryDirectory() as tmp:
        inp = Path(tmp) / "in.jsonl"
        out = Path(tmp) / "out.jsonl"
        with inp.open("w") as f:
            for r in rows:
                f.write(json.dumps(r) + "\n")
        stats = filter_heuristic(inp, out, threshold=5.0)
        kept = [json.loads(l) for l in out.open()]
        assert len(kept) == 1
        assert "sort_list" in kept[0]["output"]
        assert stats["removed_low_quality"] >= 1


def test_filter_stream_matches_batch():
    rows = [
        {"instruction": "ok", "output": "ok"},
        {
            "instruction": "Write Python to compute the factorial of n with docstring",
            "output": (
                "Here is a complete Python function to compute factorial with "
                "type hints and a docstring:\n\n"
                "```python\n"
                "def factorial(n: int) -> int:\n"
                '    """Return n! for non-negative n."""\n'
                "    if n <= 1:\n"
                "        return 1\n"
                "    return n * factorial(n - 1)\n"
                "```"
            ),
        },
    ]
    stream_kept = list(filter_stream_heuristic(iter(rows), threshold=4.0))
    assert len(stream_kept) == 1


# ── difficulty ─────────────────────────────────────────────────────────────


def test_score_difficulty_returns_unit_interval():
    rows = [
        {"instruction": "x", "output": "y"},
        {
            "instruction": "Write a function to compute factorial",
            "output": "def f(n):\n    return n * f(n-1)",
        },
        {
            "instruction": "Implement a complex graph algorithm with BFS and shortest path",
            "output": "```python\nimport heapq\n\ndef shortest_path(graph, start):\n    distances = {node: float('inf') for node in graph}\n    distances[start] = 0\n    pq = [(0, start)]\n    while pq:\n        d, u = heapq.heappop(pq)\n        if d > distances[u]:\n            continue\n        for v, w in graph[u]:\n            nd = d + w\n            if nd < distances[v]:\n                distances[v] = nd\n                heapq.heappush(pq, (nd, v))\n    return distances\n```",
        },
    ]
    for r in rows:
        s = score_difficulty(r)
        assert 0.0 <= s <= 1.0


def test_difficulty_bands_classify():
    bands = DifficultyBands(easy_max=0.4, intermediate_max=0.7)
    assert bands.classify(0.1) == "easy"
    assert bands.classify(0.5) == "intermediate"
    assert bands.classify(0.9) == "advanced"


def test_annotate_difficulty_adaptive_balances():
    rows = [
        {"instruction": f"task {i}", "output": "x" * (50 + i * 100)} for i in range(30)
    ]
    with tempfile.TemporaryDirectory() as tmp:
        inp = Path(tmp) / "in.jsonl"
        out = Path(tmp) / "out.jsonl"
        with inp.open("w") as f:
            for r in rows:
                f.write(json.dumps(r) + "\n")
        stats = annotate_difficulty(inp, out, adaptive=True)
        annotated = [json.loads(l) for l in out.open()]
        assert all("difficulty" in r for r in annotated)
        assert all("difficulty_score" in r for r in annotated)
        dist = stats["difficulty_distribution"]
        assert all(v >= 5 for v in dist.values())


# ── curator end-to-end ────────────────────────────────────────────────────


def test_curate_runs_on_local_synthetic():
    cfg = CurateConfig(
        sources=["local_synthetic"],
        output_dir=Path(tempfile.mkdtemp()) / "curated",
        target_train_size=30,
        heuristic_threshold=4.0,
    )
    manifest = curate(cfg)
    assert manifest["train_rows"] > 0
    assert manifest["val_rows"] >= 0
    assert manifest["eval_rows"] >= 0
    assert (cfg.output_dir / "train.jsonl").exists()
    assert (cfg.output_dir / "manifest.json").exists()


def test_source_registry_has_core_sources():
    expected = {"hf_codealpaca", "hf_magicoder", "hf_evol_code", "local_synthetic"}
    assert expected.issubset(SOURCE_REGISTRY.keys())
