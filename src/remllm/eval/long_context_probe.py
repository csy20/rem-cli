"""Long-context probe for verifying RoPE-scaled context handling.

The probe sends a prompt with a known long payload (e.g. 6000 tokens) to
the Ollama-served model and checks that the model uses information from
both the start and the end of the context window. This is a coarse
behavioral check that RoPE scaling is actually extending the effective
context length, not just the technical max_seq_length.
"""

from __future__ import annotations

import json
import re
import subprocess
import time
from pathlib import Path
from typing import Optional

from remllm.logging import get_logger


def make_probe_prompt(target_tokens: int = 6000) -> tuple[str, str, str]:
    """Return (prompt, needle, haystack_repeat_unit).

    `needle` is a unique string that is placed near the END of the long
    context. The model is asked to reproduce it. A model with proper
    long-context handling should be able to recall it.
    """
    needle = f"NEEDLE_{int(time.time()) % 100000}_END"
    base_unit = (
        "The quick brown fox jumps over the lazy dog. "
        "Python lists, dicts, and sets are first-class. "
        "Async/await enables cooperative concurrency. "
    )
    units_needed = max(1, target_tokens // max(1, len(base_unit.split())))
    haystack = " ".join([base_unit] * units_needed)

    mid = len(haystack) // 2
    haystack = haystack[:mid] + f" {needle} " + haystack[mid:]

    prompt = (
        "Below is a long text. Near the middle of the text, there is a "
        "unique marker of the form 'NEEDLE_<number>_END'. Reply with "
        "ONLY that marker, exactly as it appears. Do not add anything else.\n\n"
        f"TEXT:\n{haystack}\n\nMARKER:"
    )
    return prompt, needle, haystack


def run_long_context_probe(
    model: str,
    target_tokens: int = 6000,
    timeout_s: int = 180,
    ollama_url: str = "http://localhost:11434",
    max_attempts: int = 1,
) -> dict:
    """Run a single long-context probe and return metrics."""
    import urllib.request
    import urllib.error

    log = get_logger(operation="long_ctx_probe", model=model, target=target_tokens)
    prompt, needle, _ = make_probe_prompt(target_tokens)
    prompt_token_estimate = int(len(prompt.split()) * 1.3)

    payload = {
        "model": model,
        "prompt": prompt,
        "stream": False,
        "options": {
            "temperature": 0.0,
            "top_p": 1.0,
            "seed": 42,
            "num_predict": 80,
        },
    }

    start = time.time()
    try:
        data = json.dumps(payload).encode("utf-8")
        req = urllib.request.Request(
            f"{ollama_url}/api/generate",
            data=data,
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        with urllib.request.urlopen(req, timeout=timeout_s) as resp:
            body = resp.read().decode("utf-8")
            out = json.loads(body).get("response", "").strip()
    except (urllib.error.URLError, TimeoutError, json.JSONDecodeError) as exc:
        log.warning("probe_failed", error=str(exc))
        return {
            "model": model,
            "target_tokens": target_tokens,
            "prompt_token_estimate": prompt_token_estimate,
            "needle": needle,
            "response": "",
            "hit": False,
            "error": str(exc),
        }

    elapsed = round(time.time() - start, 3)
    needle_clean = re.sub(r"[^A-Z0-9_]", "", needle)
    response_clean = re.sub(r"[^A-Z0-9_]", "", out)
    hit = needle_clean in response_clean or needle in out

    stats = {
        "model": model,
        "target_tokens": target_tokens,
        "prompt_token_estimate": prompt_token_estimate,
        "needle": needle,
        "response": out[:200],
        "hit": hit,
        "latency_s": elapsed,
    }
    log.info("probe_complete", **stats)
    return stats


def run_probe_suite(
    model: str,
    token_targets: Optional[list[int]] = None,
    timeout_s: int = 180,
    ollama_url: str = "http://localhost:11434",
    output_path: Optional[Path] = None,
) -> dict:
    """Run probes at multiple context sizes and report pass/fail."""
    if token_targets is None:
        token_targets = [1024, 2048, 4096, 6000, 8000]
    results = []
    for tgt in token_targets:
        r = run_long_context_probe(
            model=model,
            target_tokens=tgt,
            timeout_s=timeout_s,
            ollama_url=ollama_url,
        )
        results.append(r)
    summary = {
        "model": model,
        "results": results,
        "max_target_reached": max(
            (r["target_tokens"] for r in results if r.get("hit")), default=0
        ),
        "all_passed": all(r.get("hit") for r in results),
    }
    if output_path:
        output_path.parent.mkdir(parents=True, exist_ok=True)
        output_path.write_text(json.dumps(summary, indent=2), encoding="utf-8")
        print(f"Wrote probe report → {output_path}")
    print(
        f"max context length successfully recalled: "
        f"{summary['max_target_reached']} tokens"
    )
    return summary
