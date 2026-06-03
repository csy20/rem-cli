"""Regression tests for Day 2-5 v0.3.0 modules: DPO generator, difficulty, long-context probe, benchmarks."""

import json
import sys
from pathlib import Path
from unittest.mock import patch

import pytest

from remllm.data.dpo_generator import (
    CODE_KEYWORDS,
    _is_code_prompt,
    _build_prompt,
    _score_candidate,
)
from remllm.data.difficulty import (
    DifficultyBands,
    annotate_difficulty,
    score_difficulty,
)
from remllm.data.ollama_client import (
    DEFAULT_OLLAMA_URL,
    GenerationParams,
    ollama_generate,
    ollama_health,
    ollama_sample_n,
)
from remllm.eval.benchmark_harness import (
    BenchmarkResult,
    check_python_solution,
    evaluate_humaneval,
    evaluate_mbpp,
    load_humaneval,
    load_mbpp,
)
from remllm.eval.long_context_probe import (
    make_probe_prompt,
    run_long_context_probe,
    run_probe_suite,
)


# ── DPO generator helpers ─────────────────────────────────────────────────


def test_is_code_prompt_positive():
    assert _is_code_prompt({"instruction": "Write a function to add two numbers"})
    assert _is_code_prompt({"instruction": "Implement a quick sort algorithm"})
    assert _is_code_prompt({"instruction": "Fix the bug in the regex match"})


def test_is_code_prompt_negative():
    assert not _is_code_prompt({"instruction": "Explain the meaning of life"})
    assert not _is_code_prompt({"instruction": "What is the capital of France?"})


def test_build_prompt_with_input():
    row = {"instruction": "i", "input": "ctx"}
    out = _build_prompt(row)
    assert "i" in out and "ctx" in out


def test_build_prompt_without_input():
    row = {"instruction": "i", "input": ""}
    out = _build_prompt(row)
    assert out == "i"


def test_score_candidate_returns_composite():
    row = {
        "instruction": "Write a function to add two numbers",
        "output": "def add(a,b): return a+b",
    }
    response = "```python\ndef add(a, b):\n    return a + b\n```"
    metrics = _score_candidate(row, response)
    assert "composite" in metrics
    assert 0.0 <= metrics["composite"] <= 1.5
    assert metrics["executable"]["executable_checked"] == 1


def test_score_candidate_exec_bonus():
    row = {"instruction": "Write Python code", "output": ""}
    good = "```python\ndef f():\n    return 42\n```"
    bad = "This is not valid Python code at all 12345"
    good_metrics = _score_candidate(row, good)
    bad_metrics = _score_candidate(row, bad)
    assert good_metrics["composite"] > bad_metrics["composite"]


# ── Difficulty scorer ─────────────────────────────────────────────────────


def test_score_difficulty_short_is_low():
    s = score_difficulty({"instruction": "x", "output": "y"})
    assert s < 0.3


def test_score_difficulty_complex_python_is_higher():
    complex_py = {
        "instruction": "Implement a complex graph traversal algorithm",
        "output": (
            "```python\n"
            "from collections import deque\n"
            "def bfs(graph, start):\n"
            "    visited = set([start])\n"
            "    queue = deque([start])\n"
            "    while queue:\n"
            "        node = queue.popleft()\n"
            "        for neighbor in graph[node]:\n"
            "            if neighbor not in visited:\n"
            "                visited.add(neighbor)\n"
            "                queue.append(neighbor)\n"
            "    return visited\n"
            "```"
        ),
    }
    s = score_difficulty(complex_py)
    assert s > 0.2


def test_annotate_difficulty_writes_manifest():
    import tempfile

    rows = [
        {"instruction": "x", "output": "y"},
        {
            "instruction": "Write code",
            "output": "```python\ndef f():\n    return 1\n```",
        },
    ]
    with tempfile.TemporaryDirectory() as tmp:
        inp = Path(tmp) / "in.jsonl"
        out = Path(tmp) / "out.jsonl"
        with inp.open("w") as f:
            for r in rows:
                f.write(json.dumps(r) + "\n")
        stats = annotate_difficulty(inp, out, adaptive=True)
        assert stats["input_rows"] == 2
        out_rows = [json.loads(l) for l in out.open()]
        assert all("difficulty" in r for r in out_rows)


# ── Ollama client ──────────────────────────────────────────────────────────


def test_generation_params_dataclass():
    p = GenerationParams(model="m", prompt="p", temperature=0.5)
    assert p.model == "m" and p.temperature == 0.5


def test_ollama_health_returns_bool():
    result = ollama_health()
    assert isinstance(result, bool)


@pytest.mark.skipif(
    not ollama_health(),
    reason="Ollama not running",
)
def test_long_context_probe_returns_dict():
    result = run_long_context_probe(
        model="rem-coder:latest", target_tokens=256, timeout_s=180
    )
    assert "hit" in result
    assert "needle" in result
    if "error" in result:
        pytest.skip(f"Ollama too slow for probe: {result['error']}")
    assert "latency_s" in result


def test_probe_suite_filters_incomplete_results():
    if not ollama_health():
        pytest.skip("Ollama not running")
    summary = run_probe_suite(
        model="rem-coder:latest", token_targets=[512, 1024], timeout_s=60
    )
    assert "max_target_reached" in summary
    assert "results" in summary
    assert all("hit" in r for r in summary["results"])


# ── Benchmark harness ─────────────────────────────────────────────────────


def test_load_humaneval(tmp_path: Path):
    he = tmp_path / "he.jsonl"
    he.write_text(
        json.dumps(
            {
                "task_id": "HE/1",
                "prompt": "def f():\n    pass\n",
                "test": "assert True",
                "entry_point": "f",
            }
        )
        + "\n"
    )
    tasks = load_humaneval(he)
    assert len(tasks) == 1
    assert tasks[0]["entry_point"] == "f"


def test_load_mbpp(tmp_path: Path):
    mb = tmp_path / "mb.jsonl"
    mb.write_text(
        json.dumps(
            {
                "task_id": 1,
                "text": "compute factorial",
                "code": "def f(): return 1",
                "test_list": ["assert True"],
            }
        )
        + "\n"
    )
    tasks = load_mbpp(mb)
    assert len(tasks) == 1
    assert "test_list" in tasks[0]


def test_check_python_solution_pass():
    code = "def add(a, b):\n    return a + b\n"
    test = "assert add(1, 2) == 3\n"
    passed, detail = check_python_solution(code, test, timeout=10)
    assert passed is True


def test_check_python_solution_fail():
    code = "def add(a, b):\n    return a - b\n"
    test = "assert add(1, 2) == 3\n"
    passed, _ = check_python_solution(code, test, timeout=10)
    assert passed is False


def test_check_python_solution_syntax_error():
    code = "def add(a, b)\n    return a + b\n"  # missing colon
    test = "pass"
    passed, detail = check_python_solution(code, test, timeout=10)
    assert passed is False
    assert "syntax" in detail.lower()


def test_benchmark_result_to_dict():
    r = BenchmarkResult(benchmark_name="humaneval", model_name="m", total=10, passed=3)
    r.pass_at_1 = 0.3
    d = r.to_dict()
    assert d["benchmark"] == "humaneval"
    assert d["pass_at_1"] == 0.3
